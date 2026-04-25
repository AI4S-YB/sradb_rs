//! Captures real responses from NCBI/ENA/OpenAI for use in offline tests.
//!
//! Usage:
//!     cargo run -p capture-fixtures -- info
//!     cargo run -p capture-fixtures -- metadata SRP174132
//!     cargo run -p capture-fixtures -- save-esearch SRP174132
//!     cargo run -p capture-fixtures -- save-esummary SRP174132

use std::path::PathBuf;
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
    Metadata {
        accession: String,
        #[arg(long, default_value_t = 20)]
        retmax: u32,
    },
    /// Capture an esearch response and write it to tests/data/ncbi/esearch_<accession>.json.
    SaveEsearch {
        accession: String,
        #[arg(long, default_value_t = 500)]
        retmax: u32,
    },
    /// Capture an esummary response (uses esearch first to get `WebEnv`) and write it to
    /// tests/data/ncbi/esummary_<accession>.xml.
    SaveEsummary {
        accession: String,
        #[arg(long, default_value_t = 500)]
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
        Cmd::Metadata { accession, retmax } => run_metadata_dump(&accession, retmax).await,
        Cmd::SaveEsearch { accession, retmax } => save_esearch(&accession, retmax).await,
        Cmd::SaveEsummary { accession, retmax } => save_esummary(&accession, retmax).await,
    }
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .join("tests/data/ncbi")
}

fn make_client(cfg: &sradb_core::ClientConfig) -> anyhow::Result<HttpClient> {
    let ncbi_rps = if cfg.has_api_key() { 10 } else { 3 };
    Ok(HttpClient::new(ncbi_rps, 8, 5, Duration::from_secs(30))?)
}

async fn esearch_raw(
    client: &HttpClient,
    cfg: &sradb_core::ClientConfig,
    accession: &str,
    retmax: u32,
) -> anyhow::Result<String> {
    let url = format!("{}/esearch.fcgi", cfg.ncbi_base_url);
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "sra"),
        ("term", accession),
        ("retmode", "json"),
        ("retmax", &retmax_s),
        ("usehistory", "y"),
    ];
    if let Some(ref k) = cfg.api_key {
        q.push(("api_key", k));
    }
    Ok(client.get_text("esearch", Service::Ncbi, &url, &q).await?)
}

async fn esummary_raw(
    client: &HttpClient,
    cfg: &sradb_core::ClientConfig,
    webenv: &str,
    query_key: &str,
    retmax: u32,
) -> anyhow::Result<String> {
    let url = format!("{}/esummary.fcgi", cfg.ncbi_base_url);
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "sra"),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", "0"),
        ("retmax", &retmax_s),
    ];
    if let Some(ref k) = cfg.api_key {
        q.push(("api_key", k));
    }
    Ok(client.get_text("esummary", Service::Ncbi, &url, &q).await?)
}

async fn save_esearch(accession: &str, retmax: u32) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let body = esearch_raw(&client, &cfg, accession, retmax).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("esearch_{accession}.json"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}

async fn save_esummary(accession: &str, retmax: u32) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let esearch_body = esearch_raw(&client, &cfg, accession, retmax).await?;
    let v: serde_json::Value = serde_json::from_str(&esearch_body)?;
    let webenv = v["esearchresult"]["webenv"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("esearch returned no webenv"))?;
    let query_key = v["esearchresult"]["querykey"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("esearch returned no querykey"))?;
    let body = esummary_raw(&client, &cfg, webenv, query_key, retmax).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("esummary_{accession}.xml"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}

async fn run_metadata_dump(accession: &str, retmax: u32) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let esearch_body = esearch_raw(&client, &cfg, accession, retmax).await?;
    let v: serde_json::Value = serde_json::from_str(&esearch_body)?;
    let result = &v["esearchresult"];
    let count = result["count"].as_str().unwrap_or("0");
    let webenv = result["webenv"].as_str().unwrap_or("");
    let query_key = result["querykey"].as_str().unwrap_or("");
    println!("=== esearch (db=sra, term={accession}) ===");
    println!("count    = {count}");
    println!("WebEnv   = {webenv}");
    println!("querykey = {query_key}");
    if webenv.is_empty() {
        anyhow::bail!("esearch returned no WebEnv");
    }
    let body = esummary_raw(&client, &cfg, webenv, query_key, retmax).await?;
    println!(
        "=== esummary (first {} chars of {} total) ===",
        body.len().min(4000),
        body.len()
    );
    println!("{}", &body[..body.len().min(4000)]);
    Ok(())
}
