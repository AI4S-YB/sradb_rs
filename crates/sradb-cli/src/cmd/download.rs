//! `sradb download <ACCESSION>... [--out-dir ...] [-j N]` handler.

use std::path::PathBuf;

use clap::Args;
use sradb_core::download::{DownloadItem, DownloadPlan};
use sradb_core::{ClientConfig, MetadataOpts, SraClient};

#[derive(Args, Debug)]
pub struct DownloadArgs {
    /// One or more SRA accessions (SRP / SRX / SRR / GSE / GSM).
    #[arg(required = true)]
    pub accessions: Vec<String>,

    /// Output directory.
    #[arg(long, default_value = "./sradb_downloads")]
    pub out_dir: PathBuf,

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
            // Prefer ENA HTTPS URLs.
            for url in &row.run.urls.ena_fastq_http {
                let filename = url.rsplit('/').next().unwrap_or("download");
                let dest = args
                    .out_dir
                    .join(&row.run.study_accession)
                    .join(&row.run.experiment_accession)
                    .join(filename);
                items.push(DownloadItem {
                    url: url.clone(),
                    dest_path: dest,
                    expected_size: None,
                });
            }
        }
    }

    if items.is_empty() {
        eprintln!("no ENA fastq URLs found for the given accessions");
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
    bar.set_message(format!("parallelism={}", args.parallelism));

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
