//! Captures real responses from NCBI/ENA/OpenAI for use in offline tests.
//!
//! Usage examples (filled out as later slices need them):
//!     cargo run -p capture-fixtures -- ncbi-esummary --db sra --term SRP016501
//!     cargo run -p capture-fixtures -- ena-filereport --accession SRR057511

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "capture-fixtures",
    about = "Dev tool: capture real-API responses for offline tests."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Sanity check: print the configured base URLs and exit.
    Info,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Info => {
            let cfg = sradb_core::ClientConfig::default();
            println!("ncbi_base_url = {}", cfg.ncbi_base_url);
            println!("ena_base_url  = {}", cfg.ena_base_url);
            println!("has_api_key   = {}", cfg.has_api_key());
            Ok(())
        }
    }
}
