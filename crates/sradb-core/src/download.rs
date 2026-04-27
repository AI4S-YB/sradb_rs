//! Parallel HTTP downloads with `Range`-based resume.
//!
//! Slice 6 implements HTTP/HTTPS only. FTP, SRA prefetch, Aspera, and md5
//! verification are deferred.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

use crate::error::{Result, SradbError};

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
    if item.dest_path.exists() {
        // Already downloaded.
        return Ok(0);
    }
    if let Some(parent) = item.dest_path.parent() {
        fs::create_dir_all(parent).await.map_err(SradbError::Io)?;
    }
    let part_path = part_path(&item.dest_path);
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

    let mut written: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|source| SradbError::Http {
            endpoint: "download",
            source,
        })?;
        file.write_all(&chunk).await.map_err(SradbError::Io)?;
        written += chunk.len() as u64;
    }
    file.flush().await.map_err(SradbError::Io)?;
    drop(file);

    fs::rename(&part_path, &item.dest_path)
        .await
        .map_err(SradbError::Io)?;
    Ok(written)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

/// Execute a download plan with bounded parallelism.
pub async fn download_plan(
    http: &reqwest::Client,
    plan: &DownloadPlan,
    parallelism: usize,
) -> DownloadReport {
    let parallelism = parallelism.max(1);
    let semaphore = Arc::new(Semaphore::new(parallelism));
    let http = http.clone();

    let mut futures = FuturesUnordered::new();
    for item in &plan.items {
        let semaphore = semaphore.clone();
        let http = http.clone();
        let item = item.clone();
        futures.push(async move {
            let _permit = semaphore.acquire().await.expect("semaphore not closed");
            let res = download_one(&http, &item).await;
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
                tracing::warn!("download failed for {}: {e}", item.url);
                report.failed += 1;
            }
        }
    }
    report
}
