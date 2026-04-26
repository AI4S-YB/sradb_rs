//! `sradb search [filters] [--format ...]` handler.

use std::io::{self, Write};

use clap::Args;
use sradb_core::search::SearchQuery;
use sradb_core::{ClientConfig, SraClient};

use crate::output::{self, Format};

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Free-text query.
    #[arg(long)]
    pub query: Option<String>,

    /// Organism scientific name (e.g. "Homo sapiens").
    #[arg(long)]
    pub organism: Option<String>,

    /// Library strategy (e.g. RNA-Seq, ChIP-Seq, WGS).
    #[arg(long)]
    pub strategy: Option<String>,

    /// Library source (e.g. TRANSCRIPTOMIC, GENOMIC).
    #[arg(long)]
    pub source: Option<String>,

    /// Library selection (e.g. cDNA, ChIP).
    #[arg(long)]
    pub selection: Option<String>,

    /// Library layout (SINGLE or PAIRED).
    #[arg(long)]
    pub layout: Option<String>,

    /// Sequencing platform (e.g. ILLUMINA, OXFORD_NANOPORE).
    #[arg(long)]
    pub platform: Option<String>,

    /// Max results (default 20, max 500 per request).
    #[arg(long, default_value_t = 20)]
    pub max: u32,

    /// Output format.
    #[arg(long, value_enum, default_value_t = Format::Tsv)]
    pub format: Format,
}

pub async fn run(args: SearchArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let query = SearchQuery {
        query: args.query,
        organism: args.organism,
        strategy: args.strategy,
        source: args.source,
        selection: args.selection,
        layout: args.layout,
        platform: args.platform,
        max: args.max,
    };

    let rows = client.search(&query).await?;
    let stdout = io::stdout();
    let mut handle = stdout.lock();
    output::write(&rows, args.format, false, &mut handle).map_err(anyhow::Error::from)?;
    handle.flush().ok();
    Ok(())
}
