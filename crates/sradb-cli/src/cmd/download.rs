//! `sradb download <ACCESSION>... [--source ...] [--out-dir ...] [-j N]` handler.

use std::path::{Path, PathBuf};

use clap::Args;
use sradb_core::download::{DownloadItem, DownloadPlan};
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
    let total = plan.items.len() as u64;
    let bar = indicatif::ProgressBar::new(total);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files {msg}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    bar.set_message(format!(
        "source={} parallelism={}",
        args.source.as_str(),
        args.parallelism
    ));

    let report = client.download(&plan, args.parallelism).await;
    bar.set_position(total);
    bar.finish_with_message(format!(
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
