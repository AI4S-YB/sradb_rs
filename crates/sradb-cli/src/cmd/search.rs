//! `sradb search [filters] [--db ...] [--format ...]` handler.

use std::io::{self, Write};

use clap::{Args, ValueEnum};
use sradb_core::search::{EnaSearchHit, GeoSearchHit, SearchQuery, ENA_SEARCH_FIELDS};
use sradb_core::{ClientConfig, SraClient};

use crate::output::{self, Format};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Backend {
    /// NCBI SRA via Entrez (esearch + esummary on `db=sra`).
    Sra,
    /// EBI ENA portal API (`/portal/api/search?result=read_run`).
    Ena,
    /// NCBI GEO Datasets (esearch + esummary on `db=gds`).
    Geo,
}

#[derive(Args, Debug)]
pub struct SearchArgs {
    /// Backend to query.
    #[arg(long, value_enum, default_value_t = Backend::Sra)]
    pub db: Backend,

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

    /// Library selection (e.g. `cDNA`, `ChIP`).
    #[arg(long)]
    pub selection: Option<String>,

    /// Library layout (SINGLE or PAIRED).
    #[arg(long)]
    pub layout: Option<String>,

    /// Sequencing platform (e.g. ILLUMINA, `OXFORD_NANOPORE`).
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

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    match args.db {
        Backend::Sra => {
            let rows = client.search(&query).await?;
            output::write(&rows, args.format, false, &mut handle).map_err(anyhow::Error::from)?;
        }
        Backend::Ena => {
            let hits = client.search_ena(&query).await?;
            write_ena(&hits, args.format, &mut handle)?;
        }
        Backend::Geo => {
            let hits = client.search_geo(&query).await?;
            write_geo(&hits, args.format, &mut handle)?;
        }
    }
    handle.flush().ok();
    Ok(())
}

fn write_ena(hits: &[EnaSearchHit], format: Format, mut out: impl Write) -> io::Result<()> {
    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut out, hits).map_err(io::Error::other)?;
            writeln!(out)?;
        }
        Format::Ndjson => {
            for h in hits {
                serde_json::to_writer(&mut out, h).map_err(io::Error::other)?;
                writeln!(out)?;
            }
        }
        Format::Tsv => {
            writeln!(out, "{}", ENA_SEARCH_FIELDS.join("\t"))?;
            for h in hits {
                let cells: Vec<String> = ENA_SEARCH_FIELDS
                    .iter()
                    .map(|c| sanitize(&ena_cell(h, c)))
                    .collect();
                writeln!(out, "{}", cells.join("\t"))?;
            }
        }
    }
    Ok(())
}

fn ena_cell(h: &EnaSearchHit, col: &str) -> String {
    let opt = |s: &Option<String>| s.clone().unwrap_or_default();
    let opt_num = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_default();
    match col {
        "run_accession" => h.run_accession.clone(),
        "experiment_accession" => h.experiment_accession.clone(),
        "sample_accession" => h.sample_accession.clone(),
        "study_accession" => h.study_accession.clone(),
        "scientific_name" => opt(&h.scientific_name),
        "library_strategy" => opt(&h.library_strategy),
        "library_source" => opt(&h.library_source),
        "library_selection" => opt(&h.library_selection),
        "library_layout" => opt(&h.library_layout),
        "instrument_platform" => opt(&h.instrument_platform),
        "instrument_model" => opt(&h.instrument_model),
        "read_count" => opt_num(h.read_count),
        "base_count" => opt_num(h.base_count),
        "study_title" => opt(&h.study_title),
        _ => String::new(),
    }
}

const GEO_COLUMNS: &[&str] = &["accession", "entry_type", "n_samples", "sra_accession"];

fn write_geo(hits: &[GeoSearchHit], format: Format, mut out: impl Write) -> io::Result<()> {
    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut out, hits).map_err(io::Error::other)?;
            writeln!(out)?;
        }
        Format::Ndjson => {
            for h in hits {
                serde_json::to_writer(&mut out, h).map_err(io::Error::other)?;
                writeln!(out)?;
            }
        }
        Format::Tsv => {
            writeln!(out, "{}", GEO_COLUMNS.join("\t"))?;
            for h in hits {
                let cells = [
                    h.accession.clone(),
                    h.entry_type.clone(),
                    h.n_samples.map(|n| n.to_string()).unwrap_or_default(),
                    h.sra_accession.clone().unwrap_or_default(),
                ];
                writeln!(
                    out,
                    "{}",
                    cells
                        .iter()
                        .map(|s| sanitize(s))
                        .collect::<Vec<_>>()
                        .join("\t")
                )?;
            }
        }
    }
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}
