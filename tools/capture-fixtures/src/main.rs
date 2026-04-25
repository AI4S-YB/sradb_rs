//! Captures real responses from NCBI/ENA/OpenAI for use in offline tests.
//!
//! Usage examples:
//!     cargo run -p capture-fixtures -- info
//!     cargo run -p capture-fixtures -- metadata SRP174132

use std::time::Duration;

use clap::{Parser, Subcommand};
use sradb_core::http::{HttpClient, Service};

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
    /// Hit NCBI esearch + esummary for an accession and dump the raw responses.
    /// Pre-slice-2 smoke test of the HttpClient against the live API.
    Metadata {
        /// SRA accession (SRP/SRX/SRR/SRS or GSE/GSM).
        accession: String,
        /// Max records to summarize.
        #[arg(long, default_value_t = 20)]
        retmax: u32,
    },
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
        Cmd::Metadata { accession, retmax } => run_metadata(&accession, retmax).await,
    }
}

async fn run_metadata(accession: &str, retmax: u32) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let ncbi_rps = if cfg.has_api_key() { 10 } else { 3 };
    let client = HttpClient::new(ncbi_rps, 8, 5, Duration::from_secs(30))?;
    let api_key = cfg.api_key.clone();

    // Step 1: esearch with usehistory=y to get a (WebEnv, query_key) handle.
    let esearch_url = format!("{}/esearch.fcgi", cfg.ncbi_base_url);
    let retmax_str = retmax.to_string();
    let mut esearch_query: Vec<(&str, &str)> = vec![
        ("db", "sra"),
        ("term", accession),
        ("retmode", "json"),
        ("retmax", &retmax_str),
        ("usehistory", "y"),
    ];
    if let Some(ref k) = api_key {
        esearch_query.push(("api_key", k));
    }

    eprintln!("→ esearch?db=sra&term={accession}");
    let esearch_body: serde_json::Value = client
        .get_json("esearch", Service::Ncbi, &esearch_url, &esearch_query)
        .await?;

    let result = &esearch_body["esearchresult"];
    let count = result["count"].as_str().unwrap_or("0");
    let webenv = result["webenv"].as_str().unwrap_or("");
    let query_key = result["querykey"].as_str().unwrap_or("");

    println!("=== esearch (db=sra, term={accession}) ===");
    println!("count    = {count}");
    println!("WebEnv   = {webenv}");
    println!("querykey = {query_key}");
    if let Some(ids) = result["idlist"].as_array() {
        println!(
            "first 5 ids = {:?}",
            ids.iter()
                .take(5)
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
        );
    }
    println!();

    if webenv.is_empty() || query_key.is_empty() {
        anyhow::bail!("esearch returned no WebEnv/querykey for {accession}");
    }

    // Step 2: esummary against the (WebEnv, query_key) — XML body.
    let esummary_url = format!("{}/esummary.fcgi", cfg.ncbi_base_url);
    let mut esummary_query: Vec<(&str, &str)> = vec![
        ("db", "sra"),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", "0"),
        ("retmax", &retmax_str),
    ];
    if let Some(ref k) = api_key {
        esummary_query.push(("api_key", k));
    }

    eprintln!("→ esummary?db=sra&WebEnv=...&query_key={query_key}");
    let esummary_body = client
        .get_text("esummary", Service::Ncbi, &esummary_url, &esummary_query)
        .await?;

    println!(
        "=== esummary (first {} chars of {} total) ===",
        esummary_body.len().min(4000),
        esummary_body.len()
    );
    println!("{}", &esummary_body[..esummary_body.len().min(4000)]);

    Ok(())
}
