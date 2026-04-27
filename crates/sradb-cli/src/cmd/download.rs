//! `sradb download <ACCESSION>... [--source ...] [--out-dir ...] [-j N]` handler.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Args;
use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use sradb_core::download::{
    partial_path, DownloadEvent, DownloadItem, DownloadPlan, DownloadProgress,
};
use sradb_core::{ClientConfig, MetadataOpts, MetadataRow, SraClient};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum DownloadSource {
    /// Download NCBI SRA / SRA Lite files from NCBI.
    Ncbi,
    /// Download ENA FASTQ files from ENA/EBI.
    Ena,
}

impl DownloadSource {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Ncbi => "ncbi",
            Self::Ena => "ena",
        }
    }

    const fn empty_message(self) -> &'static str {
        match self {
            Self::Ncbi => "no NCBI SRA URLs found for the given accessions; try --source ena",
            Self::Ena => "no ENA fastq URLs found for the given accessions; try --source ncbi",
        }
    }
}

#[derive(Args, Debug)]
pub struct DownloadArgs {
    /// One or more SRA accessions (SRP / SRX / SRR / GSE / GSM).
    #[arg(required = true)]
    pub accessions: Vec<String>,

    /// Output directory.
    #[arg(long, default_value = "./sradb_downloads")]
    pub out_dir: PathBuf,

    /// Download source.
    #[arg(long, value_enum, default_value_t = DownloadSource::Ncbi)]
    pub source: DownloadSource,

    /// Parallel download workers.
    #[arg(short = 'j', long, default_value_t = 4)]
    pub parallelism: usize,
}

pub async fn run(args: DownloadArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let opts = MetadataOpts {
        detailed: true,
        enrich: false,
        page_size: 500,
    };

    let mut items: Vec<DownloadItem> = Vec::new();
    for acc in &args.accessions {
        let rows = client.metadata(acc, &opts).await?;
        for row in &rows {
            items.extend(items_for_row(row, args.source, &args.out_dir));
        }
    }

    if items.is_empty() {
        eprintln!("{}", args.source.empty_message());
        std::process::exit(1);
    }

    let plan = DownloadPlan { items };
    let (progress_ui, progress) = progress_for_plan(&plan, args.source, args.parallelism);

    let report = client
        .download_with_progress(&plan, args.parallelism, progress)
        .await;
    progress_ui.finish(format!(
        "downloaded={}, skipped={}, failed={}",
        report.completed, report.skipped, report.failed
    ));

    if report.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn items_for_row(row: &MetadataRow, source: DownloadSource, out_dir: &Path) -> Vec<DownloadItem> {
    match source {
        DownloadSource::Ncbi => row
            .run
            .urls
            .ncbi_sra
            .as_ref()
            .map(|url| {
                let fallback = format!("{}.sra", row.run.accession);
                item_for_url(row, out_dir, url, &fallback)
            })
            .into_iter()
            .collect(),
        DownloadSource::Ena => row
            .run
            .urls
            .ena_fastq_http
            .iter()
            .map(|url| item_for_url(row, out_dir, url, "download"))
            .collect(),
    }
}

fn item_for_url(
    row: &MetadataRow,
    out_dir: &Path,
    url: &str,
    fallback_filename: &str,
) -> DownloadItem {
    let filename = filename_from_url(url, fallback_filename);
    let dest = out_dir
        .join(&row.run.study_accession)
        .join(&row.run.experiment_accession)
        .join(filename);
    DownloadItem {
        url: url.to_owned(),
        dest_path: dest,
        expected_size: None,
    }
}

fn filename_from_url(url: &str, fallback: &str) -> String {
    url.rsplit('/')
        .find(|part| !part.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

#[derive(Debug, Clone, Default)]
enum FileStatus {
    #[default]
    Pending,
    Active,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone)]
struct FileProgress {
    bar: ProgressBar,
    current: u64,
    total: Option<u64>,
    retries: u64,
    status: FileStatus,
}

#[derive(Debug)]
struct ProgressState {
    files: HashMap<PathBuf, FileProgress>,
    total_files: u64,
    completed: u64,
    skipped: u64,
    failed: u64,
    retries: u64,
    resumed_bytes: u64,
    source: &'static str,
    parallelism: usize,
}

impl ProgressState {
    fn done_files(&self) -> u64 {
        self.completed + self.skipped + self.failed
    }

    fn active_files(&self) -> usize {
        self.files
            .values()
            .filter(|file| matches!(file.status, FileStatus::Active))
            .count()
    }
}

struct ProgressUi {
    _multi: MultiProgress,
    summary: ProgressBar,
    _file_bars: Vec<ProgressBar>,
}

impl ProgressUi {
    fn finish(&self, message: String) {
        self.summary.finish_with_message(message);
    }
}

fn progress_for_plan(
    plan: &DownloadPlan,
    source: DownloadSource,
    parallelism: usize,
) -> (ProgressUi, DownloadProgress) {
    let multi = MultiProgress::new();
    let summary = multi.add(ProgressBar::new_spinner());
    summary.set_style(summary_style());
    summary.enable_steady_tick(Duration::from_millis(100));

    let mut files = HashMap::new();
    let mut file_bars = Vec::with_capacity(plan.items.len());
    let mut resumed_bytes = 0;
    for item in &plan.items {
        let name = display_name(&item.dest_path);
        let dest_len = fs::metadata(&item.dest_path).map_or(0, |meta| meta.len());
        let part_len = if dest_len == 0 {
            fs::metadata(partial_path(&item.dest_path)).map_or(0, |meta| meta.len())
        } else {
            0
        };
        resumed_bytes += part_len;

        let bar = multi.add(ProgressBar::new(1));
        bar.set_style(pending_file_style());
        bar.set_prefix(name);
        if dest_len > 0 {
            bar.set_message(format!("existing {}", HumanBytes(dest_len)));
        } else if part_len > 0 {
            bar.set_message(format!("resume {} pending", HumanBytes(part_len)));
        } else {
            bar.set_message("pending");
        }
        file_bars.push(bar.clone());

        files.insert(
            item.dest_path.clone(),
            FileProgress {
                bar,
                current: dest_len.max(part_len),
                total: item.expected_size.or((dest_len > 0).then_some(dest_len)),
                retries: 0,
                status: FileStatus::Pending,
            },
        );
    }

    let state = Arc::new(Mutex::new(ProgressState {
        files,
        total_files: plan.items.len() as u64,
        completed: 0,
        skipped: 0,
        failed: 0,
        retries: 0,
        resumed_bytes,
        source: source.as_str(),
        parallelism,
    }));

    {
        let state = state.lock().expect("progress state lock poisoned");
        refresh_summary(&summary, &state);
    }

    let callback_summary = summary.clone();
    let callback_state = Arc::clone(&state);
    let progress: DownloadProgress = Arc::new(move |event| {
        let mut state = callback_state.lock().expect("progress state lock poisoned");
        match event {
            DownloadEvent::FileStarted {
                dest_path,
                already_downloaded,
                total_size,
            } => {
                if let Some(file) = state.files.get_mut(&dest_path) {
                    file.status = FileStatus::Active;
                    file.current = already_downloaded;
                    if let Some(total_size) = total_size {
                        file.total = Some(total_size.max(already_downloaded));
                    }
                    refresh_file_bar(file);
                    if already_downloaded > 0 {
                        file.bar.set_message(format!(
                            "resuming from {}",
                            HumanBytes(already_downloaded)
                        ));
                    } else {
                        file.bar.set_message("downloading");
                    }
                }
            }
            DownloadEvent::BytesWritten { dest_path, bytes } => {
                if let Some(file) = state.files.get_mut(&dest_path) {
                    file.current = file.current.saturating_add(bytes);
                    if file.total.is_some_and(|total| file.current > total) {
                        file.total = Some(file.current);
                    }
                    refresh_file_bar(file);
                }
            }
            DownloadEvent::Retrying {
                dest_path,
                resume_from,
                attempt,
                error,
            } => {
                if let Some(file) = state.files.get_mut(&dest_path) {
                    file.status = FileStatus::Active;
                    file.retries += 1;
                    file.current = file.current.max(resume_from);
                    refresh_file_bar(file);
                    file.bar.set_message(format!(
                        "retry #{} from {} ({})",
                        attempt,
                        HumanBytes(resume_from),
                        truncate(&error, 70)
                    ));
                }
                state.retries += 1;
            }
            DownloadEvent::FileCompleted { dest_path, bytes } => {
                if let Some(file) = state.files.get_mut(&dest_path) {
                    file.current = bytes;
                    file.total = Some(bytes);
                    let was_final = matches!(
                        file.status,
                        FileStatus::Done | FileStatus::Skipped | FileStatus::Failed
                    );
                    file.status = FileStatus::Done;
                    refresh_file_bar(file);
                    file.bar
                        .finish_with_message(format!("done {}", HumanBytes(bytes)));
                    if !was_final {
                        state.completed += 1;
                    }
                }
            }
            DownloadEvent::FileSkipped { dest_path, bytes } => {
                if let Some(file) = state.files.get_mut(&dest_path) {
                    file.current = bytes;
                    file.total = Some(bytes);
                    let was_final = matches!(
                        file.status,
                        FileStatus::Done | FileStatus::Skipped | FileStatus::Failed
                    );
                    file.status = FileStatus::Skipped;
                    refresh_file_bar(file);
                    file.bar
                        .finish_with_message(format!("skip existing {}", HumanBytes(bytes)));
                    if !was_final {
                        state.skipped += 1;
                    }
                }
            }
            DownloadEvent::FileFailed { dest_path, error } => {
                if let Some(file) = state.files.get_mut(&dest_path) {
                    let was_final = matches!(
                        file.status,
                        FileStatus::Done | FileStatus::Skipped | FileStatus::Failed
                    );
                    file.status = FileStatus::Failed;
                    refresh_file_bar(file);
                    file.bar
                        .abandon_with_message(format!("failed ({})", truncate(&error, 90)));
                    if !was_final {
                        state.failed += 1;
                    }
                }
            }
        }
        refresh_summary(&callback_summary, &state);
    });

    (
        ProgressUi {
            _multi: multi,
            summary,
            _file_bars: file_bars,
        },
        progress,
    )
}

fn summary_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] {msg}")
        .unwrap()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
}

fn pending_file_style() -> ProgressStyle {
    ProgressStyle::with_template("{prefix:24} {spinner:.dim} {msg}")
        .unwrap()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
}

fn active_file_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{prefix:24} {bar:32.cyan/blue} {bytes}/{total_bytes} {bytes_per_sec} eta={eta} {msg}",
    )
    .unwrap()
    .progress_chars("=> ")
}

fn refresh_summary(bar: &ProgressBar, state: &ProgressState) {
    let mut message = format!(
        "files={}/{} active={} ok={} skipped={} failed={} retries={} source={} parallelism={}",
        state.done_files(),
        state.total_files,
        state.active_files(),
        state.completed,
        state.skipped,
        state.failed,
        state.retries,
        state.source,
        state.parallelism
    );
    if state.resumed_bytes > 0 {
        let _ = write!(message, " resumed={}", HumanBytes(state.resumed_bytes));
    }
    bar.set_message(message);
}

fn refresh_file_bar(file: &FileProgress) {
    if matches!(file.status, FileStatus::Pending) {
        return;
    }
    file.bar.set_style(active_file_style());
    let total = file
        .total
        .unwrap_or_else(|| file.current.saturating_add(1))
        .max(file.current)
        .max(1);
    file.bar.set_length(total);
    file.bar.set_position(file.current.min(total));
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("download")
        .to_owned()
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use sradb_core::{Experiment, Library, Platform, Run, RunUrls, Sample, Study};

    fn fixture_row() -> MetadataRow {
        MetadataRow {
            run: Run {
                accession: "SRR1".into(),
                experiment_accession: "SRX1".into(),
                sample_accession: "SRS1".into(),
                study_accession: "SRP1".into(),
                urls: RunUrls {
                    ena_fastq_http: vec![
                        "https://ftp.sra.ebi.ac.uk/vol1/fastq/SRR1/SRR1_1.fastq.gz".into(),
                        "https://ftp.sra.ebi.ac.uk/vol1/fastq/SRR1/SRR1_2.fastq.gz".into(),
                    ],
                    ncbi_sra: Some(
                        "https://sra-download.ncbi.nlm.nih.gov/traces/sra/SRR1.sralite.1".into(),
                    ),
                    ..RunUrls::default()
                },
                ..Run::default()
            },
            experiment: Experiment {
                accession: "SRX1".into(),
                study_accession: "SRP1".into(),
                sample_accession: "SRS1".into(),
                library: Library::default(),
                platform: Platform::default(),
                ..Experiment::default()
            },
            sample: Sample {
                accession: "SRS1".into(),
                ..Sample::default()
            },
            study: Study {
                accession: "SRP1".into(),
                ..Study::default()
            },
            enrichment: None,
        }
    }

    #[test]
    fn ncbi_source_uses_one_sra_item() {
        let items = items_for_row(&fixture_row(), DownloadSource::Ncbi, Path::new("/tmp/out"));
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].url,
            "https://sra-download.ncbi.nlm.nih.gov/traces/sra/SRR1.sralite.1"
        );
        assert_eq!(
            items[0].dest_path,
            Path::new("/tmp/out/SRP1/SRX1/SRR1.sralite.1")
        );
    }

    #[test]
    fn ena_source_uses_fastq_items() {
        let items = items_for_row(&fixture_row(), DownloadSource::Ena, Path::new("/tmp/out"));
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].dest_path,
            Path::new("/tmp/out/SRP1/SRX1/SRR1_1.fastq.gz")
        );
        assert_eq!(
            items[1].dest_path,
            Path::new("/tmp/out/SRP1/SRX1/SRR1_2.fastq.gz")
        );
    }
}
