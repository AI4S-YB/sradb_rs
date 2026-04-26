//! `sradb geo matrix <GSE> [--out-dir DIR] [--parse-tsv]` handler.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use sradb_core::geo::matrix::parse_matrix_gz;
use sradb_core::{ClientConfig, SraClient};

#[derive(Args, Debug)]
pub struct GeoArgs {
    #[command(subcommand)]
    pub cmd: GeoCmd,
}

#[derive(Subcommand, Debug)]
pub enum GeoCmd {
    /// Download a GEO Series Matrix file for a GSE accession.
    Matrix {
        /// GSE accession (e.g. `GSE56924`).
        gse: String,

        /// Output directory.
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,

        /// Also write the parsed TSV next to the .gz (header + table only).
        #[arg(long, default_value_t = false)]
        parse_tsv: bool,
    },
}

pub async fn run(args: GeoArgs) -> anyhow::Result<()> {
    match args.cmd {
        GeoCmd::Matrix {
            gse,
            out_dir,
            parse_tsv,
        } => matrix_run(&gse, &out_dir, parse_tsv).await,
    }
}

async fn matrix_run(gse: &str, out_dir: &PathBuf, parse_tsv: bool) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let bytes = client.geo_matrix_download(gse).await?;

    std::fs::create_dir_all(out_dir)?;
    let gz_path = out_dir.join(format!("{gse}_series_matrix.txt.gz"));
    std::fs::write(&gz_path, &bytes)?;
    println!("wrote {} ({} bytes)", gz_path.display(), bytes.len());

    if parse_tsv {
        let parsed = parse_matrix_gz(&bytes)?;
        let tsv_path = out_dir.join(format!("{gse}_series_matrix.tsv"));
        std::fs::write(&tsv_path, parsed.data_table.as_bytes())?;
        println!(
            "wrote {} ({} bytes; {} metadata keys)",
            tsv_path.display(),
            parsed.data_table.len(),
            parsed.series_metadata.len()
        );
    }
    Ok(())
}
