//! Parallel HTTP downloads with `Range`-based resume.
//!
//! Slice 6 implements HTTP/HTTPS only. FTP, SRA prefetch, Aspera, and md5
//! verification are deferred.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;
use tokio::time::sleep;

use crate::error::{Result, SradbError};

const MAX_CONSECUTIVE_NO_PROGRESS_RETRIES: u32 = 30;
const PROGRESS_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub enum DownloadEvent {
    FileStarted {
        dest_path: PathBuf,
        already_downloaded: u64,
        total_size: Option<u64>,
    },
    BytesWritten {
        dest_path: PathBuf,
        bytes: u64,
    },
    Retrying {
        dest_path: PathBuf,
        resume_from: u64,
        attempt: u32,
        error: String,
    },
    FileCompleted {
        dest_path: PathBuf,
        bytes: u64,
    },
    FileSkipped {
        dest_path: PathBuf,
        bytes: u64,
    },
    FileFailed {
        dest_path: PathBuf,
        error: String,
    },
}

pub type DownloadProgress = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub url: String,
    pub dest_path: PathBuf,
    /// Expected size in bytes. Used for progress reporting; `None` falls back to
    /// `Content-Length` from the response.
    pub expected_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DownloadPlan {
    pub items: Vec<DownloadItem>,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadReport {
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
}

/// Download one item to its dest path. Resumes if a `.part` file is present.
///
/// On success, the `.part` file is atomically renamed to `dest_path`.
/// Returns the bytes written (excluding any pre-existing partial bytes).
pub async fn download_one(http: &reqwest::Client, item: &DownloadItem) -> Result<u64> {
    download_one_with_progress(http, item, None).await
}

pub async fn download_one_with_progress(
    http: &reqwest::Client,
    item: &DownloadItem,
    progress: Option<DownloadProgress>,
) -> Result<u64> {
    if item.dest_path.exists() {
        let bytes = fs::metadata(&item.dest_path)
            .await
            .map_err(SradbError::Io)?
            .len();
        emit(
            progress.as_ref(),
            DownloadEvent::FileSkipped {
                dest_path: item.dest_path.clone(),
                bytes,
            },
        );
        // Already downloaded.
        return Ok(0);
    }
    if let Some(parent) = item.dest_path.parent() {
        fs::create_dir_all(parent).await.map_err(SradbError::Io)?;
    }
    let part_path = part_path(&item.dest_path);
    let initial_len = file_len(&part_path).await?;
    let mut previous_len = initial_len;
    let mut no_progress_failures = 0;
    let mut total_retries = 0;

    loop {
        match download_one_attempt(http, item, &part_path, progress.as_ref()).await {
            Ok(()) => {
                let final_len = fs::metadata(&item.dest_path)
                    .await
                    .map_err(SradbError::Io)?
                    .len();
                emit(
                    progress.as_ref(),
                    DownloadEvent::FileCompleted {
                        dest_path: item.dest_path.clone(),
                        bytes: final_len,
                    },
                );
                return Ok(final_len.saturating_sub(initial_len));
            }
            Err(e) if is_retryable_download_error(&e) => {
                let current_len = file_len(&part_path).await?;
                total_retries += 1;
                if current_len > previous_len {
                    no_progress_failures = 0;
                } else {
                    no_progress_failures += 1;
                }

                if no_progress_failures > MAX_CONSECUTIVE_NO_PROGRESS_RETRIES {
                    return Err(e);
                }

                emit(
                    progress.as_ref(),
                    DownloadEvent::Retrying {
                        dest_path: item.dest_path.clone(),
                        resume_from: current_len,
                        attempt: total_retries,
                        error: e.to_string(),
                    },
                );
                tracing::info!(
                    "download interrupted for {}; retrying from byte {}: {e}",
                    item.dest_path.display(),
                    current_len
                );
                sleep(retry_delay(no_progress_failures)).await;
                previous_len = current_len;
            }
            Err(e) => return Err(e),
        }
    }
}

async fn download_one_attempt(
    http: &reqwest::Client,
    item: &DownloadItem,
    part_path: &Path,
    progress: Option<&DownloadProgress>,
) -> Result<()> {
    let resume_from = match fs::metadata(&part_path).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    let mut request = http.get(&item.url);
    request = request.header(reqwest::header::ACCEPT_ENCODING, "identity");
    if resume_from > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let resp = request.send().await.map_err(|source| SradbError::Http {
        endpoint: "download",
        source,
    })?;

    let status = resp.status();
    if !(status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT) {
        return Err(SradbError::Download {
            url: item.url.clone(),
            reason: format!("unexpected status {status}"),
        });
    }
    let effective_resume_from = if resume_from > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT
    {
        resume_from
    } else {
        0
    };
    let total_size =
        response_total_size(status, effective_resume_from, resp.headers()).or(item.expected_size);
    emit(
        progress,
        DownloadEvent::FileStarted {
            dest_path: item.dest_path.clone(),
            already_downloaded: effective_resume_from,
            total_size,
        },
    );

    let mut file = match (
        resume_from > 0,
        status == reqwest::StatusCode::PARTIAL_CONTENT,
    ) {
        (true, true) => fs::OpenOptions::new()
            .append(true)
            .open(&part_path)
            .await
            .map_err(SradbError::Io)?,
        // server didn't honor Range (or there was nothing to resume), restart from zero
        _ => fs::File::create(&part_path).await.map_err(SradbError::Io)?,
    };

    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|source| SradbError::Http {
            endpoint: "download",
            source,
        })?;
        file.write_all(&chunk).await.map_err(SradbError::Io)?;
        emit(
            progress,
            DownloadEvent::BytesWritten {
                dest_path: item.dest_path.clone(),
                bytes: chunk.len() as u64,
            },
        );
    }
    file.flush().await.map_err(SradbError::Io)?;
    drop(file);

    fs::rename(&part_path, &item.dest_path)
        .await
        .map_err(SradbError::Io)?;
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub fn partial_path(dest: &Path) -> PathBuf {
    part_path(dest)
}

async fn file_len(path: &Path) -> Result<u64> {
    match fs::metadata(path).await {
        Ok(m) => Ok(m.len()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(0),
        Err(e) => Err(SradbError::Io(e)),
    }
}

fn is_retryable_download_error(err: &SradbError) -> bool {
    match err {
        SradbError::Http { .. } => true,
        SradbError::Download { reason, .. } => {
            reason.starts_with("unexpected status 408")
                || reason.starts_with("unexpected status 429")
                || reason.starts_with("unexpected status 5")
        }
        _ => false,
    }
}

fn retry_delay(no_progress_failures: u32) -> Duration {
    if no_progress_failures == 0 {
        return PROGRESS_RETRY_DELAY;
    }
    let exponent = no_progress_failures.saturating_sub(1).min(4);
    Duration::from_millis(250 * 2_u64.pow(exponent)).min(MAX_RETRY_DELAY)
}

fn response_total_size(
    status: reqwest::StatusCode,
    resume_from: u64,
    headers: &reqwest::header::HeaderMap,
) -> Option<u64> {
    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        content_range_total(headers)
            .or_else(|| content_length(headers).map(|len| resume_from + len))
    } else {
        content_length(headers)
    }
}

fn content_length(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::CONTENT_LENGTH)?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

fn content_range_total(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::CONTENT_RANGE)?.to_str().ok()?;
    let total = value.rsplit('/').next()?;
    if total == "*" {
        return None;
    }
    total.parse().ok()
}

fn emit(progress: Option<&DownloadProgress>, event: DownloadEvent) {
    if let Some(progress) = progress {
        progress(event);
    }
}

/// Execute a download plan with bounded parallelism.
pub async fn download_plan(
    http: &reqwest::Client,
    plan: &DownloadPlan,
    parallelism: usize,
) -> DownloadReport {
    download_plan_inner(http, plan, parallelism, None).await
}

pub async fn download_plan_with_progress(
    http: &reqwest::Client,
    plan: &DownloadPlan,
    parallelism: usize,
    progress: DownloadProgress,
) -> DownloadReport {
    download_plan_inner(http, plan, parallelism, Some(progress)).await
}

async fn download_plan_inner(
    http: &reqwest::Client,
    plan: &DownloadPlan,
    parallelism: usize,
    progress: Option<DownloadProgress>,
) -> DownloadReport {
    let parallelism = parallelism.max(1);
    let semaphore = Arc::new(Semaphore::new(parallelism));
    let http = http.clone();

    let mut futures = FuturesUnordered::new();
    for item in &plan.items {
        let semaphore = semaphore.clone();
        let http = http.clone();
        let item = item.clone();
        let progress = progress.clone();
        futures.push(async move {
            let _permit = semaphore.acquire().await.expect("semaphore not closed");
            let res = download_one_with_progress(&http, &item, progress).await;
            (item, res)
        });
    }

    let mut report = DownloadReport::default();
    while let Some((item, res)) = futures.next().await {
        match res {
            Ok(0) if item.dest_path.exists() => {
                tracing::info!("skipping {} (already exists)", item.dest_path.display());
                report.skipped += 1;
            }
            Ok(_) => {
                tracing::info!("downloaded {}", item.dest_path.display());
                report.completed += 1;
            }
            Err(e) => {
                emit(
                    progress.as_ref(),
                    DownloadEvent::FileFailed {
                        dest_path: item.dest_path.clone(),
                        error: e.to_string(),
                    },
                );
                if progress.is_some() {
                    tracing::debug!("download failed for {}: {e}", item.url);
                } else {
                    tracing::warn!("download failed for {}: {e}", item.url);
                }
                report.failed += 1;
            }
        }
    }
    report
}
