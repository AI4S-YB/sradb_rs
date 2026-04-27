//! `sradb metadata <ACCESSION>...` handler.

use std::io::{self, Write};

use clap::Args;
use sradb_core::{ClientConfig, MetadataOpts, SraClient};

use crate::output::{self, Format};

#[derive(Args, Debug)]
pub struct MetadataArgs {
    /// One or more accessions (SRP / SRX / SRR / SRS / GSE / GSM).
    #[arg(required = true)]
    pub accessions: Vec<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Tsv)]
    pub format: Format,

    /// Fetch detailed metadata: sample attributes, NCBI/S3/GS download URLs,
    /// ENA fastq URLs.
    #[arg(long, default_value_t = false)]
    pub detailed: bool,

    /// Enrich each row with LLM-extracted ontology fields (organ, tissue, etc.).
    /// Requires `OPENAI_API_KEY` env var. Optionally `OPENAI_BASE_URL` and `OPENAI_MODEL`.
    #[arg(long, default_value_t = false)]
    pub enrich: bool,

    /// Page size for esummary calls (max 500 per NCBI eUtils policy).
    #[arg(long, default_value_t = 500)]
    pub page_size: u32,
}

pub async fn run(args: MetadataArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let opts = MetadataOpts {
        detailed: args.detailed,
        enrich: args.enrich,
        page_size: args.page_size,
    };

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    let results = client.metadata_many(&args.accessions, &opts).await;
    let mut all_rows: Vec<sradb_core::MetadataRow> = Vec::new();
    let mut first_error: Option<sradb_core::SradbError> = None;
    for (acc, res) in args.accessions.iter().zip(results) {
        match res {
            Ok(rows) => all_rows.extend(rows),
            Err(e) => {
                eprintln!("error fetching metadata for {acc}: {e}");
                if first_error.is_none() {
                    first_error = Some(e);
                }
            }
        }
    }

    output::write(&all_rows, args.format, args.detailed, &mut handle)
        .map_err(anyhow::Error::from)?;
    handle.flush().ok();

    // If everything failed, surface the first error so main can classify the
    // exit code (e.g. enrichment auth failure → 3).
    if all_rows.is_empty() {
        if let Some(err) = first_error {
            return Err(anyhow::Error::new(err));
        }
    }
    Ok(())
}
