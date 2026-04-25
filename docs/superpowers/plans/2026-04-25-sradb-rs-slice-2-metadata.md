# sradb-rs Slice 2: Default `metadata` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land `sradb metadata <ACCESSION>` (default columns, no `--detailed`) end-to-end: NCBI esearch+esummary → quick-xml parsing → typed `MetadataRow` structs → TSV / JSON / NDJSON output.

**Architecture:** New modules under `sradb-core/src/`: `model.rs` (typed structs), `ncbi/` (esearch+esummary wrappers), `parse/` (eSummaryResult XML + the encoded inner ExpXml fragment), `metadata.rs` (orchestrator). The orchestrator chains `esearch (db=sra, term=ACCESSION, usehistory=y)` → `esummary (WebEnv, query_key)` → parse XML → typed rows. CLI gets a `metadata` subcommand and `output.rs` writers.

**Tech Stack:** `quick-xml` (event-streaming XML parser), `csv` (deferred to slice 3 for runinfo), existing `HttpClient` + `SraClient` from slice 1, `wiremock` + `insta` for tests.

**Reference:** Spec at `docs/superpowers/specs/2026-04-25-sradb-rs-design.md`. Slice 1 foundation: branch `slice-1-foundation`, tag `slice-1-foundation` (commit `400f850` or current branch tip).

---

## Background: the eSummaryResult shape

We confirmed against live NCBI for `SRP174132` that the response shape is:

```xml
<eSummaryResult>
  <DocSum>
    <Id>6986726</Id>
    <Item Name="ExpXml" Type="String">&lt;Summary&gt;&lt;Title&gt;...&lt;/Title&gt;&lt;Platform instrument_model="..."&gt;ILLUMINA&lt;/Platform&gt;&lt;Statistics total_runs="1" total_spots="..." total_bases="..." total_size="..."/&gt;&lt;/Summary&gt;&lt;Submitter acc="..." center_name="..." .../&gt;&lt;Experiment acc="SRX..." name="..." status="public"/&gt;&lt;Study acc="SRP..." name="..."/&gt;&lt;Organism taxid="9606" ScientificName="Homo sapiens"/&gt;&lt;Sample acc="SRS..." name=""/&gt;&lt;Instrument ILLUMINA="..."/&gt;&lt;Library_descriptor&gt;&lt;LIBRARY_STRATEGY&gt;RNA-Seq&lt;/LIBRARY_STRATEGY&gt;&lt;LIBRARY_SOURCE&gt;TRANSCRIPTOMIC&lt;/LIBRARY_SOURCE&gt;&lt;LIBRARY_SELECTION&gt;cDNA&lt;/LIBRARY_SELECTION&gt;&lt;LIBRARY_LAYOUT&gt;&lt;PAIRED/&gt;&lt;/LIBRARY_LAYOUT&gt;&lt;LIBRARY_CONSTRUCTION_PROTOCOL&gt;...&lt;/LIBRARY_CONSTRUCTION_PROTOCOL&gt;&lt;/Library_descriptor&gt;&lt;Bioproject&gt;PRJNA511021&lt;/Bioproject&gt;&lt;Biosample&gt;SAMN10621858&lt;/Biosample&gt;</Item>
    <Item Name="Runs" Type="String">&lt;Run acc="SRR..." total_spots="..." total_bases="..." load_done="true" is_public="true"/&gt;</Item>
    <Item Name="ExtLinks" Type="String"></Item>
    <Item Name="CreateDate" Type="String">2019/11/21</Item>
    <Item Name="UpdateDate" Type="String">2018/12/20</Item>
  </DocSum>
  <DocSum>...</DocSum>
</eSummaryResult>
```

**Critical wrinkle:** the `ExpXml` and `Runs` Item content is **XML-encoded XML** (every `<` is `&lt;`, etc.). After quick-xml decodes the Item text, the result is a fragment of XML (no single root). Slice 2's parser handles this by wrapping the decoded fragment with a synthetic root before re-parsing.

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/model.rs` | Public typed structs: `Study`, `Experiment`, `Library`, `LibraryLayout`, `Platform`, `Sample`, `Run`, `RunUrls`, `Enrichment`, `MetadataRow`, `MetadataOpts` |
| `crates/sradb-core/src/lib.rs` | Add module declarations and re-exports |
| `crates/sradb-core/src/parse/mod.rs` | Module root for parsers |
| `crates/sradb-core/src/parse/esummary.rs` | Parse `<eSummaryResult>` → `Vec<RawDocSum>` (with raw `exp_xml` and `runs` strings) |
| `crates/sradb-core/src/parse/exp_xml.rs` | Parse the decoded ExpXml fragment + Runs fragment into typed parts |
| `crates/sradb-core/src/ncbi/mod.rs` | Module root + `EsearchResult` shared type |
| `crates/sradb-core/src/ncbi/esearch.rs` | Async `esearch(...)` wrapper, returns `EsearchResult` |
| `crates/sradb-core/src/ncbi/esummary.rs` | Async `esummary_with_history(...)` wrapper, returns raw XML String |
| `crates/sradb-core/src/metadata.rs` | Orchestrator: `fetch_metadata(...)` chains esearch → esummary → parse → assemble |
| `crates/sradb-core/src/client.rs` | (modify) Add `metadata` and `metadata_many` methods to `SraClient` |
| `crates/sradb-core/tests/metadata_e2e.rs` | Wiremock-driven end-to-end test of the orchestrator |
| `crates/sradb-core/tests/snapshots/` | `insta` golden snapshots (auto-created on first run) |
| `crates/sradb-cli/src/main.rs` | (modify) register `metadata` subcommand |
| `crates/sradb-cli/src/cmd.rs` | New module file declaring `pub mod metadata` |
| `crates/sradb-cli/src/cmd/metadata.rs` | `metadata` subcommand handler |
| `crates/sradb-cli/src/output.rs` | TSV / JSON / NDJSON writers for `Vec<MetadataRow>` |
| `tools/capture-fixtures/src/main.rs` | (modify) Add `ncbi-esearch` and `ncbi-esummary` subcommands that save responses to `tests/data/ncbi/` |
| `tests/data/ncbi/esearch_SRP174132.json` | Captured esearch response |
| `tests/data/ncbi/esummary_SRP174132.xml` | Captured esummary response |

---

## Task 1: Public model types ✅

**Files:**
- Create: `crates/sradb-core/src/model.rs`
- Modify: `crates/sradb-core/src/lib.rs`

- [ ] **Step 1: Write `model.rs`**

```rust
//! Public typed structs returned by the metadata API.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryLayout {
    Single { length: Option<u32> },
    Paired { nominal_length: Option<u32>, nominal_sdev: Option<f32_serde::F32Eq> },
    Unknown,
}

#[allow(non_snake_case)]
mod f32_serde {
    use serde::{Deserialize, Serialize};

    /// `f32` wrapper that derives `Eq` (because we use `Option<f32>`-ish in PartialEq tests
    /// and want to keep `LibraryLayout: Eq`). Equality is bitwise on the `to_bits` representation.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct F32Eq(pub f32);

    impl PartialEq for F32Eq {
        fn eq(&self, other: &Self) -> bool {
            self.0.to_bits() == other.0.to_bits()
        }
    }
    impl Eq for F32Eq {}
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Library {
    pub strategy: Option<String>,
    pub source: Option<String>,
    pub selection: Option<String>,
    pub layout: Option<LibraryLayout>,
    pub construction_protocol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Platform {
    pub name: Option<String>,
    pub instrument_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Study {
    pub accession: String,
    pub title: Option<String>,
    pub abstract_: Option<String>,
    pub bioproject: Option<String>,
    pub geo_accession: Option<String>,
    pub pmids: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Experiment {
    pub accession: String,
    pub title: Option<String>,
    pub study_accession: String,
    pub sample_accession: String,
    pub design_description: Option<String>,
    pub library: Library,
    pub platform: Platform,
    pub geo_accession: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Sample {
    pub accession: String,
    pub title: Option<String>,
    pub biosample: Option<String>,
    pub organism_taxid: Option<u32>,
    pub organism_name: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunUrls {
    pub ena_fastq_http: Vec<String>,
    pub ena_fastq_ftp: Vec<String>,
    pub ncbi_sra: Option<String>,
    pub s3: Option<String>,
    pub gs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Run {
    pub accession: String,
    pub experiment_accession: String,
    pub sample_accession: String,
    pub study_accession: String,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub total_size: Option<u64>,
    pub published: Option<String>,
    pub urls: RunUrls,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Enrichment {
    pub organ: Option<String>,
    pub tissue: Option<String>,
    pub anatomical_system: Option<String>,
    pub cell_type: Option<String>,
    pub disease: Option<String>,
    pub sex: Option<String>,
    pub development_stage: Option<String>,
    pub assay: Option<String>,
    pub organism: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataRow {
    pub run: Run,
    pub experiment: Experiment,
    pub sample: Sample,
    pub study: Study,
    pub enrichment: Option<Enrichment>,
}

#[derive(Debug, Clone, Default)]
pub struct MetadataOpts {
    /// Slice 3 enables this. Slice 2 ignores it (always defaults to false).
    pub detailed: bool,
    /// Slice 7 enables this.
    pub enrich: bool,
    /// Pagination page size for esummary calls.
    pub page_size: u32,
}

impl MetadataOpts {
    #[must_use]
    pub fn new() -> Self {
        Self { detailed: false, enrich: false, page_size: 500 }
    }
}
```

The `f32_serde` module exists because `f32` doesn't implement `Eq`, and we want `LibraryLayout: Eq` to keep `MetadataRow: Eq`. This is overkill for slice 2 (no f32 fields are populated yet — `LibraryLayout::Paired` is constructed only in slice 3), but defining it now avoids retrofitting later.

- [ ] **Step 2: Update `lib.rs`**

Replace the contents of `crates/sradb-core/src/lib.rs` with:

```rust
//! sradb-core — core types, HTTP client, and parsers for the sradb-rs project.
//!
//! See `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` for the full spec.

pub mod accession;
pub mod client;
pub mod error;
pub mod http;
pub mod metadata;
pub mod model;
pub mod ncbi;
pub mod parse;

pub use accession::{Accession, AccessionKind, ParseAccessionError};
pub use client::{ClientConfig, SraClient};
pub use error::{Result, SradbError};
pub use model::{
    Enrichment, Experiment, Library, LibraryLayout, MetadataOpts, MetadataRow, Platform, Run,
    RunUrls, Sample, Study,
};
```

The new `pub mod` declarations will fail to compile until Tasks 2-4 create the corresponding files. To keep slice 1's tests green between tasks, create empty stubs:

- [ ] **Step 3: Create empty stubs for the new modules**

```bash
mkdir -p crates/sradb-core/src/parse crates/sradb-core/src/ncbi
```

```rust
// crates/sradb-core/src/parse/mod.rs
//! Stub. Filled in Tasks 3-7.
```

```rust
// crates/sradb-core/src/ncbi/mod.rs
//! Stub. Filled in Task 9.
```

```rust
// crates/sradb-core/src/metadata.rs
//! Stub. Filled in Task 11.
```

- [ ] **Step 4: Add tests for `model.rs`**

Append to `crates/sradb-core/src/model.rs`:

```rust

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_row_round_trips_through_json() {
        let row = MetadataRow {
            run: Run {
                accession: "SRR8361601".into(),
                experiment_accession: "SRX5172107".into(),
                sample_accession: "SRS4179725".into(),
                study_accession: "SRP174132".into(),
                total_spots: Some(38_671_668),
                total_bases: Some(11_678_843_736),
                total_size: Some(5_132_266_976),
                published: None,
                urls: RunUrls::default(),
            },
            experiment: Experiment {
                accession: "SRX5172107".into(),
                title: Some("GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq".into()),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                design_description: None,
                library: Library {
                    strategy: Some("RNA-Seq".into()),
                    source: Some("TRANSCRIPTOMIC".into()),
                    selection: Some("cDNA".into()),
                    layout: Some(LibraryLayout::Paired { nominal_length: None, nominal_sdev: None }),
                    construction_protocol: None,
                },
                platform: Platform {
                    name: Some("ILLUMINA".into()),
                    instrument_model: Some("Illumina HiSeq 2000".into()),
                },
                geo_accession: None,
            },
            sample: Sample {
                accession: "SRS4179725".into(),
                title: None,
                biosample: Some("SAMN10621858".into()),
                organism_taxid: Some(9606),
                organism_name: Some("Homo sapiens".into()),
                attributes: Default::default(),
            },
            study: Study {
                accession: "SRP174132".into(),
                title: Some("ARID1A is a critical regulator of luminal identity ...".into()),
                abstract_: None,
                bioproject: Some("PRJNA511021".into()),
                geo_accession: None,
                pmids: vec![],
            },
            enrichment: None,
        };

        let json = serde_json::to_string(&row).unwrap();
        let back: MetadataRow = serde_json::from_str(&json).unwrap();
        assert_eq!(row, back);
    }
}
```

- [ ] **Step 5: Run tests and check the workspace builds**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test -p sradb-core --lib model`
Expected: 1 test, PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-core/src
git commit -m "feat(core): add public metadata model types (Run, Experiment, Sample, Study, MetadataRow)"
```

---

## Task 2: Capture-fixtures: ncbi-esearch and ncbi-esummary save subcommands ✅

**Files:**
- Modify: `tools/capture-fixtures/src/main.rs`

We need real captured fixtures before writing the parser tests, so this task comes early.

- [ ] **Step 1: Read current main.rs**

The capture-fixtures binary already has `info` and `metadata <accession>` subcommands. Read the file to understand the existing structure.

- [ ] **Step 2: Add two new subcommands that save responses to `tests/data/ncbi/`**

Replace the `Cmd` enum and `main` body with:

```rust
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
    /// Capture an esummary response (uses esearch first to get WebEnv) and write it to
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
    let webenv = v["esearchresult"]["webenv"].as_str()
        .ok_or_else(|| anyhow::anyhow!("esearch returned no webenv"))?;
    let query_key = v["esearchresult"]["querykey"].as_str()
        .ok_or_else(|| anyhow::anyhow!("esearch returned no querykey"))?;
    let body = esummary_raw(&client, &cfg, webenv, query_key, retmax).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("esummary_{accession}.xml"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}

// run_metadata_dump is the existing slice-1 smoke test, unchanged. Keep as-is.
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
    println!("=== esummary (first {} chars of {} total) ===", body.len().min(4000), body.len());
    println!("{}", &body[..body.len().min(4000)]);
    Ok(())
}
```

The existing `run_metadata` from slice 1 is renamed to `run_metadata_dump` and refactored to share the helper functions.

- [ ] **Step 3: Build**

Run: `cargo build -p capture-fixtures 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 4: Capture the SRP174132 fixtures**

Run:
```bash
cargo run --quiet -p capture-fixtures -- save-esearch SRP174132
cargo run --quiet -p capture-fixtures -- save-esummary SRP174132
```
Expected: prints `wrote tests/data/ncbi/esearch_SRP174132.json (... bytes)` and `wrote tests/data/ncbi/esummary_SRP174132.xml (...)`.

Verify the files exist:
```bash
ls -la tests/data/ncbi/
```
Expected: two files, both non-empty (esearch JSON ~1-2KB, esummary XML ~10KB).

- [ ] **Step 5: Commit**

```bash
git add tools/capture-fixtures/src/main.rs tests/data/ncbi/
git commit -m "feat(tools): save-esearch and save-esummary commands; capture SRP174132 fixtures"
```

---

## Task 3: Parse the outer eSummaryResult XML ✅

**Files:**
- Create: `crates/sradb-core/src/parse/esummary.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs`

The outer XML wraps `<DocSum>` blocks containing five `<Item>` children. We extract `Id`, `ExpXml`, `Runs`, `CreateDate`, `UpdateDate` as raw strings.

- [ ] **Step 1: Write the parser and tests**

Replace `crates/sradb-core/src/parse/mod.rs` with:

```rust
//! Parsers for NCBI / ENA response payloads.

pub mod esummary;
```

Create `crates/sradb-core/src/parse/esummary.rs`:

```rust
//! Parser for the outer `<eSummaryResult>` envelope returned by NCBI esummary.
//!
//! Each `<DocSum>` carries five `<Item>` children. Slice 2 needs `ExpXml` and
//! `Runs` (both XML-encoded XML fragments — kept as raw strings here and
//! decoded by `parse::exp_xml` in Task 5).

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Result, SradbError};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawDocSum {
    pub id: String,
    pub exp_xml: String,
    pub runs: String,
    pub create_date: Option<String>,
    pub update_date: Option<String>,
}

const CONTEXT: &str = "esummary";

/// Parse the `<eSummaryResult>` body into a list of raw doc-sums.
///
/// `body` is the full XML response body (including the XML preamble).
pub fn parse(body: &str) -> Result<Vec<RawDocSum>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);

    let mut docs: Vec<RawDocSum> = Vec::new();
    let mut current: Option<RawDocSum> = None;
    let mut current_item_name: Option<String> = None;
    let mut current_text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return Err(SradbError::Xml { context: CONTEXT, source: e });
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"DocSum" => {
                        current = Some(RawDocSum::default());
                    }
                    b"Item" => {
                        let mut item_name: Option<String> = None;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"Name" {
                                item_name = Some(
                                    attr.unescape_value()
                                        .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?
                                        .into_owned(),
                                );
                            }
                        }
                        current_item_name = item_name;
                        current_text.clear();
                    }
                    b"Id" => {
                        current_text.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                let text = e
                    .unescape()
                    .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                current_text.push_str(&text);
            }
            Ok(Event::CData(e)) => {
                let text = std::str::from_utf8(e.as_ref()).map_err(|err| SradbError::Parse {
                    endpoint: CONTEXT,
                    message: format!("CDATA not utf-8: {err}"),
                })?;
                current_text.push_str(text);
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"DocSum" => {
                        if let Some(d) = current.take() {
                            docs.push(d);
                        }
                    }
                    b"Item" => {
                        if let (Some(item_name), Some(d)) = (current_item_name.take(), current.as_mut()) {
                            match item_name.as_str() {
                                "ExpXml" => d.exp_xml = std::mem::take(&mut current_text),
                                "Runs" => d.runs = std::mem::take(&mut current_text),
                                "CreateDate" => d.create_date = Some(std::mem::take(&mut current_text)),
                                "UpdateDate" => d.update_date = Some(std::mem::take(&mut current_text)),
                                _ => current_text.clear(),
                            }
                        }
                    }
                    b"Id" => {
                        if let Some(d) = current.as_mut() {
                            d.id = std::mem::take(&mut current_text);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(docs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<eSummaryResult>
  <DocSum>
    <Id>123</Id>
    <Item Name="ExpXml" Type="String">&lt;Summary&gt;&lt;Title&gt;hi&lt;/Title&gt;&lt;/Summary&gt;</Item>
    <Item Name="Runs" Type="String">&lt;Run acc="SRR1"/&gt;</Item>
    <Item Name="ExtLinks" Type="String"></Item>
    <Item Name="CreateDate" Type="String">2024/01/02</Item>
    <Item Name="UpdateDate" Type="String">2024/02/03</Item>
  </DocSum>
</eSummaryResult>"#;

    #[test]
    fn parses_one_docsum() {
        let docs = parse(SAMPLE).unwrap();
        assert_eq!(docs.len(), 1);
        let d = &docs[0];
        assert_eq!(d.id, "123");
        assert!(d.exp_xml.contains("<Summary>"), "exp_xml decoded: {}", d.exp_xml);
        assert!(d.exp_xml.contains("<Title>hi</Title>"));
        assert!(d.runs.contains(r#"<Run acc="SRR1"/>"#));
        assert_eq!(d.create_date.as_deref(), Some("2024/01/02"));
        assert_eq!(d.update_date.as_deref(), Some("2024/02/03"));
    }

    #[test]
    fn parses_real_srp174132_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/esummary_SRP174132.xml"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-esummary SRP174132` first");
        let docs = parse(&body).unwrap();
        assert!(!docs.is_empty(), "should have at least 1 docsum");
        for d in &docs {
            assert!(!d.id.is_empty());
            assert!(d.exp_xml.contains("<Study"), "ExpXml should contain <Study>; got: {}", &d.exp_xml[..d.exp_xml.len().min(200)]);
            assert!(d.runs.contains("<Run "), "Runs should contain <Run>");
        }
    }
}
```

The `parses_real_srp174132_fixture` test depends on `sradb-fixtures` being a dev-dep of `sradb-core`, which it already is from slice 1.

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --lib parse::esummary`
Expected: 2 tests, PASS. The second test reads `tests/data/ncbi/esummary_SRP174132.xml` saved in Task 2.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/esummary.rs
git commit -m "feat(parse): parse outer eSummaryResult XML into RawDocSum list"
```

---

## Task 4: Parse Summary, Submitter, Experiment, Study, Organism, Sample, Instrument ✅

**Files:**
- Create: `crates/sradb-core/src/parse/exp_xml.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs` (add `pub mod exp_xml;`)

The decoded `ExpXml` content is a fragment with no single root. We wrap it with a synthetic `<Root>` and parse the children.

- [ ] **Step 1: Add the module to parse::mod**

Modify `crates/sradb-core/src/parse/mod.rs`:

```rust
//! Parsers for NCBI / ENA response payloads.

pub mod esummary;
pub mod exp_xml;
```

- [ ] **Step 2: Write the parser**

Create `crates/sradb-core/src/parse/exp_xml.rs`:

```rust
//! Parser for the decoded `ExpXml` fragment (and Runs fragment) returned by NCBI esummary.
//!
//! These fragments are not single-rooted XML — they are a sequence of sibling
//! elements. We wrap them with a synthetic `<Root>` before feeding to quick-xml.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Result, SradbError};
use crate::model::{Experiment, Library, LibraryLayout, Platform, Sample, Study};

const CONTEXT: &str = "esummary_exp_xml";

/// Combined payload extracted from one `ExpXml` fragment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpXmlData {
    pub experiment_title: Option<String>,
    pub experiment_accession: String,
    pub experiment_status: Option<String>,
    pub study_accession: String,
    pub study_title: Option<String>,
    pub sample_accession: String,
    pub sample_name: Option<String>,
    pub bioproject: Option<String>,
    pub biosample: Option<String>,
    pub organism_taxid: Option<u32>,
    pub organism_name: Option<String>,
    pub platform: Platform,
    pub library: Library,
    pub total_runs: Option<u32>,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub total_size: Option<u64>,
}

/// Parse one ExpXml fragment.
pub fn parse(fragment: &str) -> Result<ExpXmlData> {
    let wrapped = format!("<Root>{fragment}</Root>");
    let mut reader = Reader::from_str(&wrapped);
    reader.config_mut().trim_text(true);

    let mut data = ExpXmlData::default();
    let mut buf = Vec::new();
    let mut text_target: Option<TextTarget> = None;
    let mut text_buf = String::new();
    let mut in_library_descriptor = false;
    let mut in_library_layout = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SradbError::Xml { context: CONTEXT, source: e }),
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                // Note: in the real eSummary payloads, the elements that we set
                // `text_target` on (Title, Platform, Bioproject, Biosample,
                // LIBRARY_*) ALWAYS arrive as Event::Start (they have content).
                // Attribute-only elements like <Statistics ... />, <Experiment ... />,
                // <Sample ... />, <Organism ... />, <PAIRED/> arrive as Event::Empty.
                // Sharing the arm is safe because the attribute-extraction code below
                // is correct for both, and no real Empty event matches a text_target tag.
                match e.name().as_ref() {
                    b"Title" if !in_library_descriptor => {
                        text_buf.clear();
                        text_target = Some(TextTarget::ExperimentTitle);
                    }
                    b"Platform" => {
                        data.platform.name = None;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"instrument_model" {
                                let v = attr.unescape_value()
                                    .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?
                                    .into_owned();
                                data.platform.instrument_model = Some(v);
                            }
                        }
                        text_buf.clear();
                        text_target = Some(TextTarget::PlatformName);
                    }
                    b"Statistics" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"total_runs" => data.total_runs = val.parse().ok(),
                                b"total_spots" => data.total_spots = val.parse().ok(),
                                b"total_bases" => data.total_bases = val.parse().ok(),
                                b"total_size" => data.total_size = val.parse().ok(),
                                _ => {}
                            }
                        }
                    }
                    b"Experiment" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"acc" => data.experiment_accession = val.into_owned(),
                                b"status" => data.experiment_status = Some(val.into_owned()),
                                b"name" => {
                                    if data.experiment_title.is_none() {
                                        data.experiment_title = Some(val.into_owned());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"Study" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"acc" => data.study_accession = val.into_owned(),
                                b"name" => data.study_title = Some(val.into_owned()),
                                _ => {}
                            }
                        }
                    }
                    b"Sample" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"acc" => data.sample_accession = val.into_owned(),
                                b"name" => {
                                    let s = val.into_owned();
                                    if !s.is_empty() {
                                        data.sample_name = Some(s);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"Organism" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"taxid" => data.organism_taxid = val.parse().ok(),
                                b"ScientificName" => data.organism_name = Some(val.into_owned()),
                                _ => {}
                            }
                        }
                    }
                    b"Bioproject" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::Bioproject);
                    }
                    b"Biosample" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::Biosample);
                    }
                    b"Library_descriptor" => in_library_descriptor = true,
                    b"LIBRARY_STRATEGY" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibStrategy);
                    }
                    b"LIBRARY_SOURCE" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibSource);
                    }
                    b"LIBRARY_SELECTION" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibSelection);
                    }
                    b"LIBRARY_LAYOUT" => in_library_layout = true,
                    b"PAIRED" if in_library_layout => {
                        data.library.layout = Some(LibraryLayout::Paired { nominal_length: None, nominal_sdev: None });
                    }
                    b"SINGLE" if in_library_layout => {
                        data.library.layout = Some(LibraryLayout::Single { length: None });
                    }
                    b"LIBRARY_CONSTRUCTION_PROTOCOL" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibProtocol);
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if text_target.is_some() {
                    let s = e.unescape().map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                    text_buf.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                match e.name().as_ref() {
                    b"Library_descriptor" => in_library_descriptor = false,
                    b"LIBRARY_LAYOUT" => in_library_layout = false,
                    _ => {}
                }
                if let Some(target) = text_target.take() {
                    let value = std::mem::take(&mut text_buf);
                    let value = value.trim().to_owned();
                    let value_opt = if value.is_empty() { None } else { Some(value) };
                    match target {
                        TextTarget::ExperimentTitle => data.experiment_title = value_opt,
                        TextTarget::PlatformName => data.platform.name = value_opt,
                        TextTarget::Bioproject => data.bioproject = value_opt,
                        TextTarget::Biosample => data.biosample = value_opt,
                        TextTarget::LibStrategy => data.library.strategy = value_opt,
                        TextTarget::LibSource => data.library.source = value_opt,
                        TextTarget::LibSelection => data.library.selection = value_opt,
                        TextTarget::LibProtocol => data.library.construction_protocol = value_opt,
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(data)
}

/// Project an `ExpXmlData` into the public `Experiment`, `Study`, `Sample` types.
#[must_use]
pub fn project(data: ExpXmlData) -> (Experiment, Study, Sample) {
    let study = Study {
        accession: data.study_accession.clone(),
        title: data.study_title,
        abstract_: None,
        bioproject: data.bioproject,
        geo_accession: None,
        pmids: vec![],
    };
    let experiment = Experiment {
        accession: data.experiment_accession.clone(),
        title: data.experiment_title,
        study_accession: data.study_accession.clone(),
        sample_accession: data.sample_accession.clone(),
        design_description: None,
        library: data.library,
        platform: data.platform,
        geo_accession: None,
    };
    let sample = Sample {
        accession: data.sample_accession,
        title: data.sample_name,
        biosample: data.biosample,
        organism_taxid: data.organism_taxid,
        organism_name: data.organism_name,
        attributes: Default::default(),
    };
    (experiment, study, sample)
}

#[derive(Debug, Clone, Copy)]
enum TextTarget {
    ExperimentTitle,
    PlatformName,
    Bioproject,
    Biosample,
    LibStrategy,
    LibSource,
    LibSelection,
    LibProtocol,
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAGMENT: &str = r#"<Summary><Title>GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq</Title><Platform instrument_model="Illumina HiSeq 2000">ILLUMINA</Platform><Statistics total_runs="1" total_spots="38671668" total_bases="11678843736" total_size="5132266976" load_done="true" cluster_name="public"/></Summary><Submitter acc="SRA826111" center_name="GEO"/><Experiment acc="SRX5172107" ver="1" status="public" name="GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq"/><Study acc="SRP174132" name="ARID1A is a critical regulator of luminal identity and therapeutic response in oestrogen receptor-positive breast cancer (RNA-Seq)"/><Organism taxid="9606" ScientificName="Homo sapiens"/><Sample acc="SRS4179725" name=""/><Instrument ILLUMINA="Illumina HiSeq 2000"/><Library_descriptor><LIBRARY_STRATEGY>RNA-Seq</LIBRARY_STRATEGY><LIBRARY_SOURCE>TRANSCRIPTOMIC</LIBRARY_SOURCE><LIBRARY_SELECTION>cDNA</LIBRARY_SELECTION><LIBRARY_LAYOUT><PAIRED/></LIBRARY_LAYOUT><LIBRARY_CONSTRUCTION_PROTOCOL>RNA was isolated using the Qiagen RNeasy kit.</LIBRARY_CONSTRUCTION_PROTOCOL></Library_descriptor><Bioproject>PRJNA511021</Bioproject><Biosample>SAMN10621858</Biosample>"#;

    #[test]
    fn parses_full_fragment() {
        let data = parse(FRAGMENT).unwrap();
        assert_eq!(data.experiment_accession, "SRX5172107");
        assert_eq!(data.study_accession, "SRP174132");
        assert_eq!(data.sample_accession, "SRS4179725");
        assert_eq!(data.organism_taxid, Some(9606));
        assert_eq!(data.organism_name.as_deref(), Some("Homo sapiens"));
        assert_eq!(data.bioproject.as_deref(), Some("PRJNA511021"));
        assert_eq!(data.biosample.as_deref(), Some("SAMN10621858"));
        assert_eq!(data.platform.name.as_deref(), Some("ILLUMINA"));
        assert_eq!(data.platform.instrument_model.as_deref(), Some("Illumina HiSeq 2000"));
        assert_eq!(data.library.strategy.as_deref(), Some("RNA-Seq"));
        assert_eq!(data.library.source.as_deref(), Some("TRANSCRIPTOMIC"));
        assert_eq!(data.library.selection.as_deref(), Some("cDNA"));
        assert!(matches!(data.library.layout, Some(LibraryLayout::Paired { .. })));
        assert_eq!(data.total_spots, Some(38_671_668));
        assert_eq!(data.total_bases, Some(11_678_843_736));
        assert_eq!(data.total_size, Some(5_132_266_976));
        assert_eq!(data.total_runs, Some(1));
    }

    #[test]
    fn project_into_public_types() {
        let data = parse(FRAGMENT).unwrap();
        let (exp, study, sample) = project(data);
        assert_eq!(exp.accession, "SRX5172107");
        assert_eq!(exp.study_accession, "SRP174132");
        assert_eq!(exp.sample_accession, "SRS4179725");
        assert_eq!(study.accession, "SRP174132");
        assert!(study.title.unwrap().starts_with("ARID1A is a critical regulator"));
        assert_eq!(sample.accession, "SRS4179725");
        assert_eq!(sample.organism_name.as_deref(), Some("Homo sapiens"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib parse::exp_xml`
Expected: 2 tests, PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/exp_xml.rs
git commit -m "feat(parse): parse decoded ExpXml fragment into typed Experiment/Study/Sample/Library"
```

---

## Task 5: Parse the Runs fragment ✅

**Files:**
- Modify: `crates/sradb-core/src/parse/exp_xml.rs` (add `parse_runs` function + tests)

The Runs Item content is `<Run acc="SRR..." total_spots="..." total_bases="..." load_done="..." is_public="..."/>` repeated 1+ times. Same wrap-with-root trick.

- [ ] **Step 1: Append the parser to `exp_xml.rs`**

Append (do not replace) to `crates/sradb-core/src/parse/exp_xml.rs`:

```rust

/// One run in the Runs fragment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawRun {
    pub accession: String,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub is_public: Option<bool>,
}

/// Parse the decoded Runs fragment into a list of raw runs.
pub fn parse_runs(fragment: &str) -> Result<Vec<RawRun>> {
    let wrapped = format!("<Root>{fragment}</Root>");
    let mut reader = Reader::from_str(&wrapped);
    reader.config_mut().trim_text(true);

    let mut runs: Vec<RawRun> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SradbError::Xml { context: CONTEXT, source: e }),
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"Run" {
                    let mut r = RawRun::default();
                    for attr in e.attributes().flatten() {
                        let val = attr.unescape_value()
                            .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                        match attr.key.as_ref() {
                            b"acc" => r.accession = val.into_owned(),
                            b"total_spots" => r.total_spots = val.parse().ok(),
                            b"total_bases" => r.total_bases = val.parse().ok(),
                            b"is_public" => r.is_public = match val.as_ref() {
                                "true" => Some(true),
                                "false" => Some(false),
                                _ => None,
                            },
                            _ => {}
                        }
                    }
                    runs.push(r);
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(runs)
}
```

Append to the test module (inside the existing `#[cfg(test)] mod tests`):

```rust

    #[test]
    fn parses_runs_single() {
        let frag = r#"<Run acc="SRR8361601" total_spots="38671668" total_bases="11678843736" load_done="true" is_public="true" cluster_name="public" static_data_available="true"/>"#;
        let runs = parse_runs(frag).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].accession, "SRR8361601");
        assert_eq!(runs[0].total_spots, Some(38_671_668));
        assert_eq!(runs[0].total_bases, Some(11_678_843_736));
        assert_eq!(runs[0].is_public, Some(true));
    }

    #[test]
    fn parses_runs_multiple() {
        let frag = r#"<Run acc="SRR1" total_spots="100"/><Run acc="SRR2" total_spots="200"/>"#;
        let runs = parse_runs(frag).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].accession, "SRR1");
        assert_eq!(runs[1].accession, "SRR2");
        assert_eq!(runs[1].total_spots, Some(200));
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --lib parse::exp_xml`
Expected: 4 tests, all PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/parse/exp_xml.rs
git commit -m "feat(parse): parse Runs fragment into RawRun list"
```

---

## Task 6: ncbi/esearch wrapper ✅

**Files:**
- Modify: `crates/sradb-core/src/ncbi/mod.rs`
- Create: `crates/sradb-core/src/ncbi/esearch.rs`

- [ ] **Step 1: Module root**

Replace `crates/sradb-core/src/ncbi/mod.rs`:

```rust
//! Wrappers for NCBI eUtils endpoints.

pub mod esearch;
pub mod esummary;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EsearchResult {
    pub count: u64,
    pub webenv: String,
    pub query_key: String,
    pub ids: Vec<String>,
}
```

- [ ] **Step 2: esearch.rs**

Create `crates/sradb-core/src/ncbi/esearch.rs`:

```rust
//! NCBI esearch wrapper.

use serde::Deserialize;

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};
use crate::ncbi::EsearchResult;

const CONTEXT: &str = "esearch";

#[derive(Debug, Deserialize)]
struct Envelope {
    esearchresult: Inner,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Inner {
    count: String,
    #[serde(rename = "webenv")]
    webenv: String,
    #[serde(rename = "querykey")]
    query_key: String,
    #[serde(default, rename = "idlist")]
    ids: Vec<String>,
}

/// Run NCBI esearch with `usehistory=y` and parse the JSON response.
pub async fn esearch(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    term: &str,
    api_key: Option<&str>,
    retmax: u32,
) -> Result<EsearchResult> {
    let url = format!("{base_url}/esearch.fcgi");
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("term", term),
        ("retmode", "json"),
        ("retmax", &retmax_s),
        ("usehistory", "y"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    let env: Envelope = http.get_json(CONTEXT, Service::Ncbi, &url, &q).await?;
    let count = env.esearchresult.count.parse::<u64>().map_err(|e| SradbError::Parse {
        endpoint: CONTEXT,
        message: format!("count `{}` not a u64: {e}", env.esearchresult.count),
    })?;
    Ok(EsearchResult {
        count,
        webenv: env.esearchresult.webenv,
        query_key: env.esearchresult.query_key,
        ids: env.esearchresult.ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn esearch_parses_typical_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esearch.fcgi"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"header":{"type":"esearch","version":"0.3"},"esearchresult":{"count":"10","retmax":"500","retstart":"0","querykey":"1","webenv":"MCID_abc123","idlist":["1","2","3"]}}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let result = esearch(&http, &server.uri(), "sra", "SRP174132", None, 500).await.unwrap();
        assert_eq!(result.count, 10);
        assert_eq!(result.webenv, "MCID_abc123");
        assert_eq!(result.query_key, "1");
        assert_eq!(result.ids, vec!["1".to_string(), "2".into(), "3".into()]);
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib ncbi::esearch`
Expected: 1 test, PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/ncbi
git commit -m "feat(ncbi): esearch wrapper returning EsearchResult"
```

---

## Task 7: ncbi/esummary wrapper ✅

**Files:**
- Create: `crates/sradb-core/src/ncbi/esummary.rs`

- [ ] **Step 1: Implementation**

Create `crates/sradb-core/src/ncbi/esummary.rs`:

```rust
//! NCBI esummary wrapper.

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT: &str = "esummary";

/// Fetch one page of esummary results using a (WebEnv, query_key) handle.
/// Returns the raw response body (XML by default for db=sra).
pub async fn esummary_with_history(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    webenv: &str,
    query_key: &str,
    retstart: u32,
    retmax: u32,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/esummary.fcgi");
    let retstart_s = retstart.to_string();
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", &retstart_s),
        ("retmax", &retmax_s),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT, Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn esummary_returns_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esummary.fcgi"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<eSummaryResult/>"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = esummary_with_history(
            &http, &server.uri(), "sra", "WE", "QK", 0, 500, None,
        ).await.unwrap();
        assert_eq!(body, "<eSummaryResult/>");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --lib ncbi::esummary`
Expected: 1 test, PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/ncbi/esummary.rs
git commit -m "feat(ncbi): esummary_with_history wrapper returning raw body"
```

---

## Task 8: metadata orchestrator ✅

**Files:**
- Modify: `crates/sradb-core/src/metadata.rs` (full rewrite — overwrites Task 1's stub)

- [ ] **Step 1: Implement the orchestrator**

Replace `crates/sradb-core/src/metadata.rs`:

```rust
//! Metadata orchestrator: chains esearch → esummary → parse → typed `MetadataRow`.
//!
//! Slice 2 implements the default (non-detailed) path. `--detailed` and `--enrich`
//! land in slices 3 and 7 respectively.

use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::model::{MetadataOpts, MetadataRow, Run, RunUrls};
use crate::ncbi::{esearch, esummary};
use crate::parse;

/// Drive the full default-metadata flow for a single accession.
///
/// Pagination: if the esearch count exceeds `opts.page_size`, esummary is called
/// repeatedly with increasing `retstart` until all rows are collected.
pub async fn fetch_metadata(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    term: &str,
    opts: &MetadataOpts,
) -> Result<Vec<MetadataRow>> {
    let page = opts.page_size.max(1);
    let result = esearch::esearch(http, ncbi_base_url, "sra", term, api_key, page).await?;
    if result.count == 0 {
        return Ok(Vec::new());
    }
    if result.webenv.is_empty() || result.query_key.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "esearch",
            message: format!("count={} but missing webenv/query_key", result.count),
        });
    }

    let mut rows: Vec<MetadataRow> = Vec::with_capacity(result.count as usize);
    let mut retstart: u32 = 0;
    let total = u32::try_from(result.count).unwrap_or(u32::MAX);
    while retstart < total {
        let body = esummary::esummary_with_history(
            http,
            ncbi_base_url,
            "sra",
            &result.webenv,
            &result.query_key,
            retstart,
            page,
            api_key,
        )
        .await?;
        let docs = parse::esummary::parse(&body)?;
        if docs.is_empty() {
            break;
        }
        for d in docs {
            rows.extend(assemble_rows(d)?);
        }
        retstart += page;
    }
    Ok(rows)
}

/// One DocSum can carry multiple `<Run>` entries (paired-end studies, etc.).
/// Emit one `MetadataRow` per run, sharing the experiment/study/sample.
fn assemble_rows(doc: parse::esummary::RawDocSum) -> Result<Vec<MetadataRow>> {
    let exp = parse::exp_xml::parse(&doc.exp_xml)?;
    let runs = parse::exp_xml::parse_runs(&doc.runs)?;
    if runs.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "esummary",
            message: format!("no <Run> in DocSum id={}", doc.id),
        });
    }
    let (experiment, study, sample) = parse::exp_xml::project(exp.clone());
    let published = doc.update_date.or(doc.create_date);
    let rows = runs
        .into_iter()
        .map(|raw_run| MetadataRow {
            run: Run {
                accession: raw_run.accession,
                experiment_accession: experiment.accession.clone(),
                sample_accession: experiment.sample_accession.clone(),
                study_accession: experiment.study_accession.clone(),
                total_spots: raw_run.total_spots.or(exp.total_spots),
                total_bases: raw_run.total_bases.or(exp.total_bases),
                total_size: exp.total_size,
                published: published.clone(),
                urls: RunUrls::default(),
            },
            experiment: experiment.clone(),
            sample: sample.clone(),
            study: study.clone(),
            enrichment: None,
        })
        .collect();
    Ok(rows)
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/metadata.rs
git commit -m "feat(core): metadata orchestrator chaining esearch+esummary+parse"
```

---

## Task 9: SraClient::metadata + metadata_many methods ✅

**Files:**
- Modify: `crates/sradb-core/src/client.rs`

- [ ] **Step 1: Read current client.rs**

Read `crates/sradb-core/src/client.rs` so the next edit is precise. The `impl SraClient { ... }` block currently has `new`, `with_config`, `with_base_urls`, and `config`.

- [ ] **Step 2: Append the metadata methods**

Append two new methods inside the existing `impl SraClient { ... }` block (before its closing `}`):

```rust

    /// Fetch metadata for one accession.
    pub async fn metadata(
        &self,
        accession: &str,
        opts: &crate::model::MetadataOpts,
    ) -> Result<Vec<crate::model::MetadataRow>> {
        crate::metadata::fetch_metadata(
            &self.http,
            &self.cfg.ncbi_base_url,
            self.cfg.api_key.as_deref(),
            accession,
            opts,
        )
        .await
    }

    /// Fetch metadata for many accessions concurrently. The returned vec is
    /// in input order; each element is the per-accession result (success or
    /// error). Failures of one accession do not abort the others.
    pub async fn metadata_many(
        &self,
        accessions: &[String],
        opts: &crate::model::MetadataOpts,
    ) -> Vec<Result<Vec<crate::model::MetadataRow>>> {
        let futures = accessions.iter().map(|a| self.metadata(a, opts));
        futures::future::join_all(futures).await
    }
```

The `dead_code` allow on `http` can now be removed because slice 2 reads it. Update the struct definition:

```rust
#[derive(Clone)]
pub struct SraClient {
    pub(crate) http: HttpClient,
    pub(crate) cfg: ClientConfig,
}
```

(Remove the `#[allow(dead_code)]` and the comment above `http`.)

- [ ] **Step 3: Add the futures dependency to sradb-core**

`futures` is already a workspace dep but not yet declared in `crates/sradb-core/Cargo.toml`. Read the file and add `futures.workspace = true` to `[dependencies]`.

- [ ] **Step 4: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS, and the dead_code warning should be gone.

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-core/src/client.rs crates/sradb-core/Cargo.toml
git commit -m "feat(core): SraClient::metadata + metadata_many"
```

---

## Task 10: End-to-end orchestrator test (wiremock + fixtures) ✅

**Files:**
- Create: `crates/sradb-core/tests/metadata_e2e.rs`

- [ ] **Step 1: Write the test**

Create `crates/sradb-core/tests/metadata_e2e.rs`:

```rust
//! End-to-end test of `SraClient::metadata` against captured fixtures served by wiremock.

use sradb_core::{ClientConfig, MetadataOpts, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn metadata_srp174132_against_fixtures() {
    let esearch_body = std::fs::read_to_string(
        sradb_fixtures::workspace_root().join("tests/data/ncbi/esearch_SRP174132.json"),
    )
    .expect("run `cargo run -p capture-fixtures -- save-esearch SRP174132` first");
    let esummary_body = std::fs::read_to_string(
        sradb_fixtures::workspace_root().join("tests/data/ncbi/esummary_SRP174132.xml"),
    )
    .expect("run `cargo run -p capture-fixtures -- save-esummary SRP174132` first");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("term", "SRP174132"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esummary_body))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: format!("{}/ena", server.uri()),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let mut rows = client.metadata("SRP174132", &MetadataOpts::new()).await.unwrap();
    rows.sort_by(|a, b| a.run.accession.cmp(&b.run.accession));

    assert!(!rows.is_empty(), "expected at least 1 row");
    for r in &rows {
        assert_eq!(r.study.accession, "SRP174132", "study accession should match");
        assert!(r.run.accession.starts_with("SRR"), "run acc: {}", r.run.accession);
        assert!(r.experiment.accession.starts_with("SRX"));
        assert!(r.sample.accession.starts_with("SRS"));
        assert_eq!(r.sample.organism_name.as_deref(), Some("Homo sapiens"));
        assert_eq!(r.sample.organism_taxid, Some(9606));
        assert_eq!(r.experiment.library.strategy.as_deref(), Some("RNA-Seq"));
    }

    insta::assert_json_snapshot!("metadata_srp174132", rows, {
        "[].run.published" => "[date]",
    });
}
```

- [ ] **Step 2: Run test (insta will record the snapshot on first run)**

Run: `cargo test -p sradb-core --test metadata_e2e 2>&1 | tail -10`

On first run, `insta` creates a `.snap.new` file but the assertion fails. Accept the snapshot:

```bash
INSTA_UPDATE=always cargo test -p sradb-core --test metadata_e2e
```

Then re-run normally to confirm the snapshot is committed:

```bash
cargo test -p sradb-core --test metadata_e2e
```
Expected: PASS.

The `[].run.published` redaction handles the fact that our parsed `published` field comes from the doc-sum's `UpdateDate`/`CreateDate`, which are stable in the fixture but better redacted.

- [ ] **Step 3: Commit (including the .snap file)**

```bash
git add crates/sradb-core/tests/metadata_e2e.rs crates/sradb-core/tests/snapshots
git commit -m "test(core): wiremock+insta e2e for SraClient::metadata against SRP174132"
```

---

## Task 11: TSV / JSON / NDJSON output writers ✅

**Files:**
- Create: `crates/sradb-cli/src/output.rs`
- Modify: `crates/sradb-cli/src/main.rs` (add `mod output;`)

- [ ] **Step 1: Write the writers**

Create `crates/sradb-cli/src/output.rs`:

```rust
//! Output writers for `Vec<MetadataRow>`.

use std::io::{self, Write};

use sradb_core::MetadataRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Tsv,
    Json,
    Ndjson,
}

const TSV_COLUMNS: &[&str] = &[
    "study_accession",
    "study_title",
    "experiment_accession",
    "experiment_title",
    "organism_taxid",
    "organism_name",
    "library_strategy",
    "library_source",
    "library_selection",
    "library_layout",
    "sample_accession",
    "sample_title",
    "biosample",
    "bioproject",
    "instrument",
    "instrument_model",
    "total_spots",
    "total_bases",
    "total_size",
    "run_accession",
    "run_total_spots",
    "run_total_bases",
];

fn cell(row: &MetadataRow, col: &str) -> String {
    use sradb_core::LibraryLayout;
    let opt_string = |s: &Option<String>| s.clone().unwrap_or_default();
    let opt_num = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_default();
    match col {
        "study_accession" => row.study.accession.clone(),
        "study_title" => opt_string(&row.study.title),
        "experiment_accession" => row.experiment.accession.clone(),
        "experiment_title" => opt_string(&row.experiment.title),
        "organism_taxid" => row.sample.organism_taxid.map(|n| n.to_string()).unwrap_or_default(),
        "organism_name" => opt_string(&row.sample.organism_name),
        "library_strategy" => opt_string(&row.experiment.library.strategy),
        "library_source" => opt_string(&row.experiment.library.source),
        "library_selection" => opt_string(&row.experiment.library.selection),
        "library_layout" => match &row.experiment.library.layout {
            Some(LibraryLayout::Single { .. }) => "SINGLE".into(),
            Some(LibraryLayout::Paired { .. }) => "PAIRED".into(),
            Some(LibraryLayout::Unknown) | None => String::new(),
        },
        "sample_accession" => row.sample.accession.clone(),
        "sample_title" => opt_string(&row.sample.title),
        "biosample" => opt_string(&row.sample.biosample),
        "bioproject" => opt_string(&row.study.bioproject),
        "instrument" => opt_string(&row.experiment.platform.name),
        "instrument_model" => opt_string(&row.experiment.platform.instrument_model),
        "total_spots" => opt_num(row.run.total_spots),
        "total_bases" => opt_num(row.run.total_bases),
        "total_size" => opt_num(row.run.total_size),
        "run_accession" => row.run.accession.clone(),
        "run_total_spots" => opt_num(row.run.total_spots),
        "run_total_bases" => opt_num(row.run.total_bases),
        _ => String::new(),
    }
}

pub fn write(rows: &[MetadataRow], format: Format, mut out: impl Write) -> io::Result<()> {
    match format {
        Format::Tsv => write_tsv(rows, &mut out),
        Format::Json => write_json(rows, &mut out),
        Format::Ndjson => write_ndjson(rows, &mut out),
    }
}

fn write_tsv<W: Write>(rows: &[MetadataRow], out: &mut W) -> io::Result<()> {
    writeln!(out, "{}", TSV_COLUMNS.join("\t"))?;
    for row in rows {
        let cells: Vec<String> = TSV_COLUMNS.iter().map(|c| sanitize_tsv(&cell(row, c))).collect();
        writeln!(out, "{}", cells.join("\t"))?;
    }
    Ok(())
}

fn sanitize_tsv(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn write_json<W: Write>(rows: &[MetadataRow], out: &mut W) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, rows).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn write_ndjson<W: Write>(rows: &[MetadataRow], out: &mut W) -> io::Result<()> {
    for row in rows {
        serde_json::to_writer(&mut *out, row).map_err(io::Error::other)?;
        writeln!(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sradb_core::{Experiment, Library, MetadataRow, Platform, Run, RunUrls, Sample, Study};

    fn fixture_row() -> MetadataRow {
        MetadataRow {
            run: Run {
                accession: "SRR8361601".into(),
                experiment_accession: "SRX5172107".into(),
                sample_accession: "SRS4179725".into(),
                study_accession: "SRP174132".into(),
                total_spots: Some(38_671_668),
                total_bases: Some(11_678_843_736),
                total_size: Some(5_132_266_976),
                published: None,
                urls: RunUrls::default(),
            },
            experiment: Experiment {
                accession: "SRX5172107".into(),
                title: Some("RNA-Seq: H1".into()),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                design_description: None,
                library: Library {
                    strategy: Some("RNA-Seq".into()),
                    source: Some("TRANSCRIPTOMIC".into()),
                    selection: Some("cDNA".into()),
                    layout: Some(sradb_core::LibraryLayout::Paired { nominal_length: None, nominal_sdev: None }),
                    construction_protocol: None,
                },
                platform: Platform {
                    name: Some("ILLUMINA".into()),
                    instrument_model: Some("Illumina HiSeq 2000".into()),
                },
                geo_accession: None,
            },
            sample: Sample {
                accession: "SRS4179725".into(),
                title: None,
                biosample: Some("SAMN10621858".into()),
                organism_taxid: Some(9606),
                organism_name: Some("Homo sapiens".into()),
                attributes: Default::default(),
            },
            study: Study {
                accession: "SRP174132".into(),
                title: Some("ARID1A study".into()),
                abstract_: None,
                bioproject: Some("PRJNA511021".into()),
                geo_accession: None,
                pmids: vec![],
            },
            enrichment: None,
        }
    }

    #[test]
    fn tsv_has_header_and_one_row() {
        let mut out = Vec::new();
        write(std::slice::from_ref(&fixture_row()), Format::Tsv, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], TSV_COLUMNS.join("\t"));
        assert!(lines[1].contains("SRP174132"));
        assert!(lines[1].contains("SRR8361601"));
        assert!(lines[1].contains("RNA-Seq"));
        assert!(lines[1].contains("PAIRED"));
    }

    #[test]
    fn json_round_trips() {
        let mut out = Vec::new();
        write(std::slice::from_ref(&fixture_row()), Format::Json, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let back: Vec<MetadataRow> = serde_json::from_str(&s).unwrap();
        assert_eq!(back, vec![fixture_row()]);
    }

    #[test]
    fn ndjson_has_one_line_per_row() {
        let row = fixture_row();
        let rows = vec![row.clone(), row];
        let mut out = Vec::new();
        write(&rows, Format::Ndjson, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2);
        for line in text.lines() {
            let _: MetadataRow = serde_json::from_str(line).unwrap();
        }
    }
}
```

- [ ] **Step 2: Add output to main.rs**

Read `crates/sradb-cli/src/main.rs`. Add `mod output;` near the top (after the doc comment and before `use`).

- [ ] **Step 3: Add `serde_json` to sradb-cli dependencies**

The output module uses `serde_json`. Edit `crates/sradb-cli/Cargo.toml` and add to `[dependencies]`:

```toml
serde_json.workspace = true
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p sradb-cli --bin sradb`
Expected: 3 tests PASS (`tsv_has_header_and_one_row`, `json_round_trips`, `ndjson_has_one_line_per_row`).

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-cli/src/output.rs crates/sradb-cli/src/main.rs crates/sradb-cli/Cargo.toml
git commit -m "feat(cli): TSV/JSON/NDJSON output writers"
```

---

## Task 12: CLI metadata subcommand ✅

**Files:**
- Create: `crates/sradb-cli/src/cmd.rs`
- Create: `crates/sradb-cli/src/cmd/metadata.rs`
- Modify: `crates/sradb-cli/src/main.rs`

- [ ] **Step 1: Create cmd module root**

Create `crates/sradb-cli/src/cmd.rs`:

```rust
//! Subcommand handlers.

pub mod metadata;
```

- [ ] **Step 2: Create the metadata handler**

Create `crates/sradb-cli/src/cmd/metadata.rs`:

```rust
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

    /// Page size for esummary calls (max 500 per NCBI eUtils policy).
    #[arg(long, default_value_t = 500)]
    pub page_size: u32,
}

pub async fn run(args: MetadataArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let opts = MetadataOpts {
        detailed: false,
        enrich: false,
        page_size: args.page_size,
    };

    let stdout = io::stdout();
    let mut handle = stdout.lock();

    let results = client.metadata_many(&args.accessions, &opts).await;
    let mut all_rows: Vec<sradb_core::MetadataRow> = Vec::new();
    let mut had_error = false;
    for (acc, res) in args.accessions.iter().zip(results) {
        match res {
            Ok(rows) => all_rows.extend(rows),
            Err(e) => {
                had_error = true;
                eprintln!("error fetching metadata for {acc}: {e}");
            }
        }
    }

    output::write(&all_rows, args.format, &mut handle).map_err(anyhow::Error::from)?;
    handle.flush().ok();

    if all_rows.is_empty() && had_error {
        std::process::exit(1);
    }
    Ok(())
}
```

- [ ] **Step 3: Wire into main.rs**

Replace `crates/sradb-cli/src/main.rs` with:

```rust
//! sradb command-line interface.

mod cmd;
mod output;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "sradb", version, about = "Query NGS metadata from SRA / ENA / GEO.", long_about = None)]
struct Cli {
    /// Increase verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print build information and exit.
    Info,
    /// Fetch metadata for one or more accessions.
    Metadata(cmd::metadata::MetadataArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Some(Cmd::Info) => {
            println!("sradb {}", env!("CARGO_PKG_VERSION"));
            println!("https://github.com/saketkc/pysradb (Rust port)");
            Ok(())
        }
        Some(Cmd::Metadata(args)) => cmd::metadata::run(args).await,
        None => {
            <Cli as clap::CommandFactory>::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

fn init_tracing(verbosity: u8) {
    use tracing_subscriber::{fmt, EnvFilter};

    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("sradb={level},sradb_core={level},sradb_cli={level}")));
    fmt().with_env_filter(filter).with_target(false).init();
}
```

The function `main` was previously `fn main() -> anyhow::Result<()>`. It's now `async fn` with `#[tokio::main]` because we await the metadata handler.

- [ ] **Step 4: Build**

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 5: Smoke-check `--help` output**

Run: `cargo run -p sradb-cli --quiet -- metadata --help 2>&1 | tail -15`
Expected: Help text including `--format` and `--page-size` flags.

- [ ] **Step 6: Live smoke test against real NCBI**

Run: `cargo run -p sradb-cli --quiet -- metadata SRP174132 --format json 2>&1 | head -50`

Expected: a JSON array. Each element is a `MetadataRow`. Run count should be 10 (matches the slice-1 smoke test).

If you don't have outbound HTTPS to `eutils.ncbi.nlm.nih.gov` available, skip this step and rely on the wiremock e2e test from Task 10.

- [ ] **Step 7: Commit**

```bash
git add crates/sradb-cli/src/cmd.rs crates/sradb-cli/src/cmd crates/sradb-cli/src/main.rs
git commit -m "feat(cli): sradb metadata <accession>... subcommand"
```

---

## Task 13: Final verification

**Files:** none changed; verification only.

- [ ] **Step 1: Build everything**

Run: `cargo build --workspace --all-targets 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 2: Run all tests**

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: PASS, total test count should be ≥30 (slice 1 had 15; slice 2 adds at least 15 more).

- [ ] **Step 3: Clippy**

Run: `RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 4: Fmt**

Run: `cargo fmt --all -- --check 2>&1 | tail -3`
Expected: PASS. If it fails, run `cargo fmt --all` and commit the formatting changes as a separate commit.

- [ ] **Step 5: Tag the slice**

```bash
git tag -a slice-2-metadata -m "Slice 2: default metadata orchestrator end-to-end (no --detailed, no --enrich)"
```

- [ ] **Step 6: Update plan checkboxes**

Edit `docs/superpowers/plans/2026-04-25-sradb-rs-slice-2-metadata.md` and mark each `## Task N: ...` heading with a trailing `✅` for the tasks that are done.

- [ ] **Step 7: Commit plan updates**

```bash
git add docs/superpowers/plans/2026-04-25-sradb-rs-slice-2-metadata.md
git commit -m "docs(plan): mark slice-2 tasks complete"
```

---

## What this slice does NOT include (intentional deferrals)

- `--detailed` flag — efetch runinfo CSV, ExperimentPackageSet XML for sample attributes, ENA fastq URL fan-out, expanded download URLs (NCBI / S3 / GS). Slice 3.
- `--enrich` flag — OpenAI chat-completions enrichment. Slice 7.
- Multi-accession dedupe — if you pass `SRP174132 SRP174132` you get duplicate rows. Acceptable for slice 2.
- ENA backend (search of metadata against ENA portal). Slice 5.
- GEO accession resolution — passing `GSE...` or `GSM...` won't work yet because eUtils db=sra rejects those. Slice 4 adds the conversion that resolves GSE→SRP first.

## Definition of done for slice 2

1. `cargo build --workspace` clean.
2. `cargo test --workspace` clean — at least 30 tests passing.
3. `cargo clippy --workspace --all-targets` with `-Dwarnings` clean.
4. `cargo fmt --all -- --check` clean.
5. `sradb metadata SRP174132 --format json` outputs 10 valid `MetadataRow` JSON entries against live NCBI.
6. `sradb metadata SRP174132 --format tsv` outputs a TSV with the documented 22-column header.
7. The wiremock + insta e2e test (`metadata_e2e.rs`) passes hermetically (no network needed).
8. `git tag slice-2-metadata` created.
