# sradb-rs Slice 3: `--detailed` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land `sradb metadata <ACCESSION> --detailed`: augment each `MetadataRow` with sample attributes (dynamic key=value pairs), NCBI/S3/GS download URLs, and ENA fastq URLs (HTTP + FTP, paired or single).

**Architecture:** Detailed mode chains three additional fetches after esummary: (a) `efetch retmode=runinfo` for total_bases / total_size / published refinement, (b) `efetch` of `EXPERIMENT_PACKAGE_SET` XML for sample attributes + SRAFiles, (c) `ENA filereport` per run for fastq URLs (fan-out concurrency = 8). All three are guarded by `opts.detailed` so default metadata is unaffected.

**Tech Stack:** existing `quick-xml` for XML, `csv` (already a workspace dep, slice-3-introduced) for runinfo, `reqwest` for ENA, `tokio::sync::Semaphore` + `futures::stream::FuturesUnordered` for fan-out.

**Reference:** Spec at `docs/superpowers/specs/2026-04-25-sradb-rs-design.md`. Slices 1-2: tags `slice-1-foundation`, `slice-2-metadata`.

---

## Background: the wire shapes

### efetch runinfo (CSV)

```
GET {ncbi}/efetch.fcgi?db=sra&WebEnv=...&query_key=...&rettype=runinfo&retmode=csv
```

Returns CSV with header. Columns include (truncated, ~30 in real responses): `Run`, `ReleaseDate`, `LoadDate`, `spots`, `bases`, `size_MB`, `Experiment`, `LibraryStrategy`, `Sample`, `BioProject`, `BioSample`, `Submission`, `Study`, ... etc. The columns we need for slice 3 augmentation:
- `Run` → match key
- `bases` → `Run.total_bases` (already populated from ExpXml `Statistics`, runinfo confirms)
- `size_MB` → `Run.total_size` (better source than ExpXml total_size which is study-aggregate)
- `ReleaseDate` → `Run.published` (replaces the Update/CreateDate fallback from default mode)

### efetch EXPERIMENT_PACKAGE_SET (XML)

```
GET {ncbi}/efetch.fcgi?db=sra&WebEnv=...&query_key=...&rettype=full&retmode=xml
```

Returns properly-rooted XML (no fragment trick needed):

```xml
<EXPERIMENT_PACKAGE_SET>
  <EXPERIMENT_PACKAGE>
    <EXPERIMENT accession="SRX5172107" ... > ... </EXPERIMENT>
    <SUBMISSION ... />
    <Organization> ... </Organization>
    <STUDY accession="SRP174132" ...> ... </STUDY>
    <SAMPLE accession="SRS4179725" ...>
      <IDENTIFIERS> ... </IDENTIFIERS>
      <TITLE>...</TITLE>
      <SAMPLE_NAME>
        <TAXON_ID>9606</TAXON_ID>
        <SCIENTIFIC_NAME>Homo sapiens</SCIENTIFIC_NAME>
      </SAMPLE_NAME>
      <SAMPLE_ATTRIBUTES>
        <SAMPLE_ATTRIBUTE><TAG>source_name</TAG><VALUE>liver</VALUE></SAMPLE_ATTRIBUTE>
        <SAMPLE_ATTRIBUTE><TAG>cell type</TAG><VALUE>hepatocyte</VALUE></SAMPLE_ATTRIBUTE>
        ...
      </SAMPLE_ATTRIBUTES>
    </SAMPLE>
    <Pool> ... </Pool>
    <RUN_SET>
      <RUN accession="SRR8361601" total_spots="..." total_bases="..." size="..." published="...">
        <IDENTIFIERS> ... </IDENTIFIERS>
        <Pool> ... </Pool>
        <SRAFiles>
          <SRAFile cluster="public" filename="SRR8361601" url="https://sra-pub-run-odp.s3.amazonaws.com/sra/..." size="..." date="..." md5="..." semantic_name="run" supertype="Original" sratoolkit="1">
            <Alternatives url="https://sra-download.ncbi.nlm.nih.gov/..." free_egress="worldwide" access_type="anonymous" org="NCBI" />
            <Alternatives url="s3://sra-pub-run-odp/sra/..." free_egress="-" access_type="aws identity" org="AWS" />
            <Alternatives url="gs://sra-pub-run-1/..." free_egress="-" access_type="gcp identity" org="GCP" />
          </SRAFile>
        </SRAFiles>
      </RUN>
    </RUN_SET>
  </EXPERIMENT_PACKAGE>
</EXPERIMENT_PACKAGE_SET>
```

We extract:
- `SAMPLE/SAMPLE_ATTRIBUTES/SAMPLE_ATTRIBUTE/{TAG,VALUE}` → `Sample.attributes` (BTreeMap)
- `RUN/SRAFiles/SRAFile/Alternatives` filtered by `org`:
  - `org="NCBI"` → `Run.urls.ncbi_sra`
  - `org="AWS"` → `Run.urls.s3`
  - `org="GCP"` → `Run.urls.gs`
- `RUN[@published]` → `Run.published` (overrides the default mode fallback)

### ENA filereport (TSV)

```
GET https://www.ebi.ac.uk/ena/portal/api/filereport?accession=SRR8361601&result=read_run&fields=fastq_ftp,fastq_md5,fastq_bytes,fastq_aspera&format=tsv
```

Returns TSV with one row per run. The `fastq_ftp` field is `;`-separated for paired-end:

```
run_accession	fastq_ftp	fastq_md5	fastq_bytes	fastq_aspera
SRR8361601	ftp.sra.ebi.ac.uk/vol1/fastq/SRR836/001/SRR8361601/SRR8361601_1.fastq.gz;ftp.sra.ebi.ac.uk/vol1/fastq/SRR836/001/SRR8361601/SRR8361601_2.fastq.gz	abc;def	123;456	era-fasp@fasp...
```

To produce HTTP URLs we prefix `https://` to each FTP path (ENA serves the same paths over HTTPS at `https://ftp.sra.ebi.ac.uk/...`).

## Scope

**In:** runinfo CSV, EXPERIMENT_PACKAGE_SET XML (sample attributes + SRAFile URLs), ENA filereport with fan-out concurrency=8, dynamic sample-attribute columns in TSV/JSON output.

**Out (deferred):**
- `study_geo_accession` / `experiment_geo_accession` — needs the accession-conversion engine, lands in slice 4.
- PMIDs — needs `srp_to_pmid` conversion, lands in slice 4 polish.
- Sample TITLE / DESCRIPTION fields from EXPERIMENT_PACKAGE_SET — pysradb default detailed doesn't expose them; only the SAMPLE_ATTRIBUTES bag.

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/parse/sample_attrs.rs` | Parse the pipe-delimited `key: value \|\| key: value` form pysradb uses for serialized sample_attribute strings (kept for downstream interop, even though we now also produce typed maps directly) |
| `crates/sradb-core/src/parse/runinfo.rs` | Parse efetch runinfo CSV into `Vec<RunInfo>` |
| `crates/sradb-core/src/parse/experiment_package.rs` | Parse EXPERIMENT_PACKAGE_SET XML into typed `ExperimentPackage` records (one per experiment) |
| `crates/sradb-core/src/parse/ena_filereport.rs` | Parse ENA filereport TSV into `Vec<EnaFilereportRow>` |
| `crates/sradb-core/src/ncbi/efetch.rs` | Async wrappers: `efetch_runinfo_with_history`, `efetch_full_xml_with_history` |
| `crates/sradb-core/src/ena.rs` | Async ENA filereport client: `fetch_filereport(run_accession)` |
| `crates/sradb-core/src/metadata.rs` | (modify) Detailed branch: chain runinfo + exp_pkg + ENA fan-out and augment rows |
| `tools/capture-fixtures/src/main.rs` | (modify) Add `save-efetch-runinfo`, `save-efetch-xml`, `save-ena-filereport` subcommands |
| `tests/data/ncbi/efetch_runinfo_SRP174132.csv` | Captured runinfo fixture |
| `tests/data/ncbi/efetch_xml_SRP174132.xml` | Captured experiment package fixture |
| `tests/data/ena/filereport_SRR8361601.tsv` | One captured ENA filereport |
| `crates/sradb-cli/src/cmd/metadata.rs` | (modify) Add `--detailed` CLI flag |
| `crates/sradb-cli/src/output.rs` | (modify) Detailed column set + dynamic sample-attribute columns |
| `crates/sradb-core/tests/metadata_detailed_e2e.rs` | New e2e test exercising the detailed path against captured fixtures |

---

## Task 1: Parse sample_attribute pipe-delimited string

**Files:**
- Create: `crates/sradb-core/src/parse/sample_attrs.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs`

This task targets pysradb's pipe-delimited form (`source_name: liver || cell type: hepatocyte`). Slice 3 sources sample attributes directly from EXPERIMENT_PACKAGE_SET XML (not from a serialized string), so this parser exists primarily as a small focused unit and for future interop.

- [ ] **Step 1: Add module to parse/mod.rs**

Read `crates/sradb-core/src/parse/mod.rs`. Append `pub mod sample_attrs;`:

```rust
//! Parsers for NCBI / ENA response payloads.

pub mod esummary;
pub mod exp_xml;
pub mod sample_attrs;
```

- [ ] **Step 2: Implement parser**

Create `crates/sradb-core/src/parse/sample_attrs.rs`:

```rust
//! Parser for pysradb-style serialized sample_attribute strings.
//!
//! Format: `key: value || key: value || key: value`. Values may contain
//! colons (`source_name: Liver: Adult`) — only the FIRST `:` separates key/value.

use std::collections::BTreeMap;

/// Parse a pipe-delimited `key: value` string.
/// Whitespace around keys and values is trimmed. Empty entries are dropped.
#[must_use]
pub fn parse(input: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for entry in input.split("||") {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((k, v)) = entry.split_once(':') {
            let key = k.trim().to_owned();
            let val = v.trim().to_owned();
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple() {
        let m = parse("source_name: liver || cell type: hepatocyte");
        assert_eq!(m.get("source_name").map(String::as_str), Some("liver"));
        assert_eq!(m.get("cell type").map(String::as_str), Some("hepatocyte"));
    }

    #[test]
    fn value_with_colon() {
        let m = parse("source_name: Liver: Adult");
        assert_eq!(m.get("source_name").map(String::as_str), Some("Liver: Adult"));
    }

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(parse("").is_empty());
        assert!(parse("   ").is_empty());
        assert!(parse(" || ").is_empty());
    }

    #[test]
    fn trims_whitespace() {
        let m = parse("  k1  :  v1  ||  k2:v2 ");
        assert_eq!(m.get("k1").map(String::as_str), Some("v1"));
        assert_eq!(m.get("k2").map(String::as_str), Some("v2"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib parse::sample_attrs`
Expected: 4 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/sample_attrs.rs
git commit -m "feat(parse): pipe-delimited sample_attribute string parser"
```

---

## Task 2: Capture detailed-mode fixtures

**Files:**
- Modify: `tools/capture-fixtures/src/main.rs` (add 3 subcommands)
- Create: `tests/data/ncbi/efetch_runinfo_SRP174132.csv`
- Create: `tests/data/ncbi/efetch_xml_SRP174132.xml`
- Create: `tests/data/ena/filereport_SRR8361601.tsv`

We capture fixtures early so subsequent parser tests can target real shapes.

- [ ] **Step 1: Read current main.rs**

The capture-fixtures binary already has `info`, `metadata`, `save-esearch`, `save-esummary`. Add three more subcommands.

- [ ] **Step 2: Add three new variants to the `Cmd` enum**

Inside the `enum Cmd { ... }` block, after `SaveEsummary { ... }`, insert:

```rust
    /// Capture an efetch runinfo response and write it to
    /// tests/data/ncbi/efetch_runinfo_<accession>.csv.
    SaveEfetchRuninfo {
        accession: String,
        #[arg(long, default_value_t = 500)]
        retmax: u32,
    },
    /// Capture an efetch retmode=xml response (EXPERIMENT_PACKAGE_SET) and write it to
    /// tests/data/ncbi/efetch_xml_<accession>.xml.
    SaveEfetchXml {
        accession: String,
        #[arg(long, default_value_t = 500)]
        retmax: u32,
    },
    /// Capture an ENA filereport for one run and write it to
    /// tests/data/ena/filereport_<run>.tsv.
    SaveEnaFilereport {
        /// SRR/ERR/DRR run accession.
        run: String,
    },
```

- [ ] **Step 3: Add three new arms to the `match cli.cmd` block**

After the `SaveEsummary` arm:

```rust
        Cmd::SaveEfetchRuninfo { accession, retmax } => save_efetch_runinfo(&accession, retmax).await,
        Cmd::SaveEfetchXml { accession, retmax } => save_efetch_xml(&accession, retmax).await,
        Cmd::SaveEnaFilereport { run } => save_ena_filereport(&run).await,
```

- [ ] **Step 4: Add the three helper functions**

Append to `tools/capture-fixtures/src/main.rs` (after the existing `save_esummary` function, before `run_metadata_dump`):

```rust
async fn efetch_raw(
    client: &HttpClient,
    cfg: &sradb_core::ClientConfig,
    webenv: &str,
    query_key: &str,
    rettype: &str,
    retmode: &str,
    retmax: u32,
) -> anyhow::Result<String> {
    let url = format!("{}/efetch.fcgi", cfg.ncbi_base_url);
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "sra"),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", "0"),
        ("retmax", &retmax_s),
        ("rettype", rettype),
        ("retmode", retmode),
    ];
    if let Some(ref k) = cfg.api_key {
        q.push(("api_key", k));
    }
    Ok(client.get_text("efetch", Service::Ncbi, &url, &q).await?)
}

async fn handle_for(client: &HttpClient, cfg: &sradb_core::ClientConfig, accession: &str, retmax: u32)
    -> anyhow::Result<(String, String)>
{
    let esearch_body = esearch_raw(client, cfg, accession, retmax).await?;
    let v: serde_json::Value = serde_json::from_str(&esearch_body)?;
    let webenv = v["esearchresult"]["webenv"].as_str()
        .ok_or_else(|| anyhow::anyhow!("esearch returned no webenv"))?
        .to_owned();
    let query_key = v["esearchresult"]["querykey"].as_str()
        .ok_or_else(|| anyhow::anyhow!("esearch returned no querykey"))?
        .to_owned();
    Ok((webenv, query_key))
}

async fn save_efetch_runinfo(accession: &str, retmax: u32) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let (webenv, query_key) = handle_for(&client, &cfg, accession, retmax).await?;
    let body = efetch_raw(&client, &cfg, &webenv, &query_key, "runinfo", "csv", retmax).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("efetch_runinfo_{accession}.csv"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}

async fn save_efetch_xml(accession: &str, retmax: u32) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let (webenv, query_key) = handle_for(&client, &cfg, accession, retmax).await?;
    let body = efetch_raw(&client, &cfg, &webenv, &query_key, "full", "xml", retmax).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("efetch_xml_{accession}.xml"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}

async fn save_ena_filereport(run: &str) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let url = format!("{}/portal/api/filereport", cfg.ena_base_url);
    let body = client
        .get_text(
            "ena_filereport",
            Service::Ena,
            &url,
            &[
                ("accession", run),
                ("result", "read_run"),
                ("fields", "fastq_ftp,fastq_md5,fastq_bytes,fastq_aspera"),
                ("format", "tsv"),
            ],
        )
        .await?;
    let workspace_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().and_then(std::path::Path::parent).expect("workspace root").to_path_buf();
    let dir = workspace_root.join("tests/data/ena");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("filereport_{run}.tsv"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}
```

Note: ENA's base URL in `ClientConfig::default()` is `https://www.ebi.ac.uk/ena`. The portal API is at `{base}/portal/api/filereport`.

- [ ] **Step 5: Build**

Run: `cargo build -p capture-fixtures 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 6: Capture the fixtures**

Network access to NCBI + ENA required.

```bash
cargo run --quiet -p capture-fixtures -- save-efetch-runinfo SRP174132
cargo run --quiet -p capture-fixtures -- save-efetch-xml SRP174132
cargo run --quiet -p capture-fixtures -- save-ena-filereport SRR8361601
```

Expected outputs:
- `wrote .../tests/data/ncbi/efetch_runinfo_SRP174132.csv (... bytes)` — likely 5-15KB, 11 lines (header + 10 runs)
- `wrote .../tests/data/ncbi/efetch_xml_SRP174132.xml (... bytes)` — likely 50-200KB, 10 EXPERIMENT_PACKAGE blocks
- `wrote .../tests/data/ena/filereport_SRR8361601.tsv (... bytes)` — likely 200-500B, 2 lines (header + 1 run)

- [ ] **Step 7: Spot-check the fixtures**

Run:
```bash
head -2 tests/data/ncbi/efetch_runinfo_SRP174132.csv
head -1 tests/data/ena/filereport_SRR8361601.tsv
grep -c "<EXPERIMENT_PACKAGE>" tests/data/ncbi/efetch_xml_SRP174132.xml
```

Expected:
- runinfo: header line starting with `Run,ReleaseDate,LoadDate,spots,bases,...` and one data line for SRR8361592 or similar
- ENA TSV: header line with `run_accession\tfastq_ftp\tfastq_md5\tfastq_bytes\tfastq_aspera`
- 10 EXPERIMENT_PACKAGE blocks

- [ ] **Step 8: Commit**

```bash
git add tools/capture-fixtures/src/main.rs tests/data/ncbi tests/data/ena
git commit -m "feat(tools): save-efetch-runinfo / save-efetch-xml / save-ena-filereport; capture SRP174132 detailed fixtures"
```

---

## Task 3: Parse efetch runinfo CSV

**Files:**
- Create: `crates/sradb-core/src/parse/runinfo.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs`

- [ ] **Step 1: Add module declaration**

```rust
// crates/sradb-core/src/parse/mod.rs
//! Parsers for NCBI / ENA response payloads.

pub mod esummary;
pub mod exp_xml;
pub mod runinfo;
pub mod sample_attrs;
```

- [ ] **Step 2: Add `csv` dep to sradb-core**

Read `crates/sradb-core/Cargo.toml`. `csv.workspace = true` is already in `[dependencies]` (added in slice 1). No change needed.

- [ ] **Step 3: Implement parser**

Create `crates/sradb-core/src/parse/runinfo.rs`:

```rust
//! Parser for the efetch retmode=runinfo CSV output.
//!
//! Real responses have ~30 columns; we only consume the four we need to refine
//! `Run` fields beyond what ExpXml provided.

use std::collections::HashMap;

use crate::error::{Result, SradbError};

const CONTEXT: &str = "efetch_runinfo";

/// Per-run augmentation extracted from runinfo CSV.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunInfo {
    pub run_accession: String,
    pub bases: Option<u64>,
    pub size_mb: Option<u64>,
    pub release_date: Option<String>,
}

/// Parse a runinfo CSV body into a map keyed by run accession.
pub fn parse(body: &str) -> Result<HashMap<String, RunInfo>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(body.as_bytes());

    let headers = reader.headers().map_err(|e| SradbError::Csv { context: CONTEXT, source: e })?
        .clone();

    let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let i_run = col("Run");
    let i_bases = col("bases");
    let i_size_mb = col("size_MB").or_else(|| col("size_mb"));
    let i_release = col("ReleaseDate");

    let mut out: HashMap<String, RunInfo> = HashMap::new();
    for record in reader.records() {
        let record = record.map_err(|e| SradbError::Csv { context: CONTEXT, source: e })?;
        let mut info = RunInfo::default();
        if let Some(i) = i_run {
            if let Some(v) = record.get(i) {
                info.run_accession = v.to_owned();
            }
        }
        if info.run_accession.is_empty() {
            continue;
        }
        if let Some(i) = i_bases {
            info.bases = record.get(i).and_then(|s| s.parse().ok());
        }
        if let Some(i) = i_size_mb {
            info.size_mb = record.get(i).and_then(|s| s.parse().ok());
        }
        if let Some(i) = i_release {
            info.release_date = record.get(i).map(str::to_owned).filter(|s| !s.is_empty());
        }
        out.insert(info.run_accession.clone(), info);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Run,ReleaseDate,LoadDate,spots,bases,spots_with_mates,avgLength,size_MB,Experiment\nSRR8361601,2018-12-20 10:00:00,2018-12-21,38671668,11678843736,0,302,4894,SRX5172107\n";

    #[test]
    fn parses_one_row() {
        let map = parse(SAMPLE).unwrap();
        assert_eq!(map.len(), 1);
        let info = map.get("SRR8361601").unwrap();
        assert_eq!(info.bases, Some(11_678_843_736));
        assert_eq!(info.size_mb, Some(4894));
        assert_eq!(info.release_date.as_deref(), Some("2018-12-20 10:00:00"));
    }

    #[test]
    fn parses_real_srp174132_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_runinfo_SRP174132.csv"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-efetch-runinfo SRP174132` first");
        let map = parse(&body).unwrap();
        assert!(!map.is_empty(), "should have ≥ 1 run");
        for (acc, info) in &map {
            assert!(acc.starts_with("SRR"));
            assert_eq!(&info.run_accession, acc);
            assert!(info.bases.is_some(), "bases should parse for {acc}");
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p sradb-core --lib parse::runinfo`
Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/runinfo.rs
git commit -m "feat(parse): efetch runinfo CSV parser"
```

---

## Task 4: Parse EXPERIMENT_PACKAGE_SET XML — sample attributes

**Files:**
- Create: `crates/sradb-core/src/parse/experiment_package.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs`

This task lands the parser skeleton + sample-attribute extraction. Task 5 extends it with SRAFile URLs.

- [ ] **Step 1: Add module to parse/mod.rs**

```rust
//! Parsers for NCBI / ENA response payloads.

pub mod esummary;
pub mod experiment_package;
pub mod exp_xml;
pub mod runinfo;
pub mod sample_attrs;
```

- [ ] **Step 2: Implement parser (sample attributes only for now)**

Create `crates/sradb-core/src/parse/experiment_package.rs`:

```rust
//! Parser for the EXPERIMENT_PACKAGE_SET XML returned by `efetch retmode=xml`.
//!
//! Slice 3 extracts: per-experiment SAMPLE_ATTRIBUTES (key/value bag) and
//! per-run SRAFile alternatives (NCBI / S3 / GS download URLs).

use std::collections::BTreeMap;
use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Result, SradbError};

const CONTEXT: &str = "efetch_xml";

/// Per-experiment data extracted from one `<EXPERIMENT_PACKAGE>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExperimentPackage {
    pub experiment_accession: String,
    pub sample_accession: String,
    pub sample_attributes: BTreeMap<String, String>,
    /// Download URLs by run accession.
    pub run_urls: HashMap<String, SraFileUrls>,
    /// Run published timestamp (overrides default-mode fallback).
    pub run_published: HashMap<String, String>,
}

/// Per-run download URLs extracted from `<SRAFiles>/<SRAFile>/<Alternatives>`.
/// Distinct from `model::RunUrls` (which also carries ENA fastq lists).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SraFileUrls {
    pub ncbi_sra: Option<String>,
    pub s3: Option<String>,
    pub gs: Option<String>,
}

/// Parse an entire EXPERIMENT_PACKAGE_SET body into one `ExperimentPackage` per
/// experiment, keyed by experiment accession.
pub fn parse(body: &str) -> Result<HashMap<String, ExperimentPackage>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut packages: HashMap<String, ExperimentPackage> = HashMap::new();
    let mut current: Option<ExperimentPackage> = None;

    // SAMPLE_ATTRIBUTE tracking
    let mut in_sample = false;
    let mut in_sample_attributes = false;
    let mut in_sample_attribute = false;
    let mut tag_text: Option<String> = None;
    let mut value_text: Option<String> = None;
    let mut text_target: Option<TextTarget> = None;
    let mut text_buf = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SradbError::Xml { context: CONTEXT, source: e }),
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e) | Event::Start(e)) => match e.name().as_ref() {
                b"EXPERIMENT_PACKAGE" => current = Some(ExperimentPackage::default()),
                b"EXPERIMENT" => {
                    if let Some(p) = current.as_mut() {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"accession" {
                                let v = attr.unescape_value()
                                    .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?
                                    .into_owned();
                                p.experiment_accession = v;
                            }
                        }
                    }
                }
                b"SAMPLE" => {
                    in_sample = true;
                    if let Some(p) = current.as_mut() {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"accession" {
                                let v = attr.unescape_value()
                                    .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?
                                    .into_owned();
                                p.sample_accession = v;
                            }
                        }
                    }
                }
                b"SAMPLE_ATTRIBUTES" if in_sample => in_sample_attributes = true,
                b"SAMPLE_ATTRIBUTE" if in_sample_attributes => {
                    in_sample_attribute = true;
                    tag_text = None;
                    value_text = None;
                }
                b"TAG" if in_sample_attribute => {
                    text_buf.clear();
                    text_target = Some(TextTarget::Tag);
                }
                b"VALUE" if in_sample_attribute => {
                    text_buf.clear();
                    text_target = Some(TextTarget::Value);
                }
                _ => {}
            },
            Ok(Event::Text(e)) => {
                if text_target.is_some() {
                    let s = e.unescape().map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                    text_buf.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                match e.name().as_ref() {
                    b"EXPERIMENT_PACKAGE" => {
                        if let Some(pkg) = current.take() {
                            if !pkg.experiment_accession.is_empty() {
                                packages.insert(pkg.experiment_accession.clone(), pkg);
                            }
                        }
                    }
                    b"SAMPLE" => in_sample = false,
                    b"SAMPLE_ATTRIBUTES" => in_sample_attributes = false,
                    b"SAMPLE_ATTRIBUTE" => {
                        if let (Some(t), Some(v), Some(p)) = (
                            tag_text.take(),
                            value_text.take(),
                            current.as_mut(),
                        ) {
                            let t = t.trim().to_owned();
                            let v = v.trim().to_owned();
                            if !t.is_empty() {
                                p.sample_attributes.insert(t, v);
                            }
                        }
                        in_sample_attribute = false;
                    }
                    _ => {}
                }
                if let Some(target) = text_target.take() {
                    let value = std::mem::take(&mut text_buf);
                    match target {
                        TextTarget::Tag => tag_text = Some(value),
                        TextTarget::Value => value_text = Some(value),
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(packages)
}

#[derive(Debug, Clone, Copy)]
enum TextTarget {
    Tag,
    Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<EXPERIMENT_PACKAGE_SET>
<EXPERIMENT_PACKAGE>
<EXPERIMENT accession="SRX5172107"/>
<SAMPLE accession="SRS4179725">
<SAMPLE_ATTRIBUTES>
<SAMPLE_ATTRIBUTE><TAG>source_name</TAG><VALUE>liver</VALUE></SAMPLE_ATTRIBUTE>
<SAMPLE_ATTRIBUTE><TAG>cell type</TAG><VALUE>hepatocyte</VALUE></SAMPLE_ATTRIBUTE>
</SAMPLE_ATTRIBUTES>
</SAMPLE>
</EXPERIMENT_PACKAGE>
</EXPERIMENT_PACKAGE_SET>"#;

    #[test]
    fn parses_sample_attributes() {
        let pkgs = parse(SAMPLE).unwrap();
        assert_eq!(pkgs.len(), 1);
        let p = &pkgs["SRX5172107"];
        assert_eq!(p.sample_accession, "SRS4179725");
        assert_eq!(p.sample_attributes.get("source_name").map(String::as_str), Some("liver"));
        assert_eq!(p.sample_attributes.get("cell type").map(String::as_str), Some("hepatocyte"));
    }

    #[test]
    fn parses_real_srp174132_fixture_sample_attrs() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_xml_SRP174132.xml"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-efetch-xml SRP174132` first");
        let pkgs = parse(&body).unwrap();
        assert!(!pkgs.is_empty(), "should have ≥ 1 package");
        for (exp, pkg) in &pkgs {
            assert!(exp.starts_with("SRX"), "experiment accession: {exp}");
            assert!(!pkg.sample_accession.is_empty(), "{exp} should have sample acc");
            // SRP174132 samples have at least source_name attribute
            // (verify the bag isn't empty)
            assert!(!pkg.sample_attributes.is_empty(), "{exp} should have sample attrs");
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p sradb-core --lib parse::experiment_package`
Expected: 2 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/experiment_package.rs
git commit -m "feat(parse): EXPERIMENT_PACKAGE_SET sample attribute parser"
```

---

## Task 5: Extend EXPERIMENT_PACKAGE_SET parser with SRAFile URLs

**Files:**
- Modify: `crates/sradb-core/src/parse/experiment_package.rs`

- [ ] **Step 1: Read current parser**

Read `crates/sradb-core/src/parse/experiment_package.rs` to find the `loop { match reader.read_event_into(...) ... }` block.

- [ ] **Step 2: Add RUN/SRAFiles state tracking**

Add new state variables in the function (after `text_buf` declaration):

```rust
    // RUN / SRAFiles tracking
    let mut current_run_acc: Option<String> = None;
    let mut current_run_published: Option<String> = None;
    let mut current_sra_file_alternatives: Vec<(String, String)> = Vec::new();  // (org, url) per run
    let mut in_run = false;
    let mut in_sra_files = false;
    let mut in_sra_file = false;
```

- [ ] **Step 3: Add new arms in the Empty/Start match**

In the existing `Ok(Event::Empty(e) | Event::Start(e))` arm's inner match, add (alphabetically):

```rust
                b"RUN" => {
                    in_run = true;
                    current_run_acc = None;
                    current_run_published = None;
                    current_sra_file_alternatives.clear();
                    for attr in e.attributes().flatten() {
                        let val = attr.unescape_value()
                            .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                        match attr.key.as_ref() {
                            b"accession" => current_run_acc = Some(val.into_owned()),
                            b"published" => current_run_published = Some(val.into_owned()),
                            _ => {}
                        }
                    }
                }
                b"SRAFiles" if in_run => in_sra_files = true,
                b"SRAFile" if in_sra_files => in_sra_file = true,
                b"Alternatives" if in_sra_file => {
                    let mut org: Option<String> = None;
                    let mut url: Option<String> = None;
                    for attr in e.attributes().flatten() {
                        let v = attr.unescape_value()
                            .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                        match attr.key.as_ref() {
                            b"org" => org = Some(v.into_owned()),
                            b"url" => url = Some(v.into_owned()),
                            _ => {}
                        }
                    }
                    if let (Some(org), Some(url)) = (org, url) {
                        current_sra_file_alternatives.push((org, url));
                    }
                }
```

- [ ] **Step 4: Add new arms in the End match**

In the existing `Ok(Event::End(e))` arm's match (before the `_ => {}` default), add:

```rust
                    b"RUN" => {
                        if let (Some(acc), Some(p)) = (current_run_acc.take(), current.as_mut()) {
                            let mut urls = SraFileUrls::default();
                            for (org, url) in current_sra_file_alternatives.drain(..) {
                                match org.as_str() {
                                    "NCBI" => urls.ncbi_sra = Some(url),
                                    "AWS" => urls.s3 = Some(url),
                                    "GCP" => urls.gs = Some(url),
                                    _ => {}
                                }
                            }
                            p.run_urls.insert(acc.clone(), urls);
                            if let Some(pub_) = current_run_published.take() {
                                p.run_published.insert(acc, pub_);
                            }
                        }
                        in_run = false;
                    }
                    b"SRAFiles" => in_sra_files = false,
                    b"SRAFile" => in_sra_file = false,
```

- [ ] **Step 5: Add tests**

Append two more tests inside the existing `mod tests`:

```rust

    #[test]
    fn parses_sra_file_alternatives() {
        const XML: &str = r#"<?xml version="1.0"?>
<EXPERIMENT_PACKAGE_SET>
<EXPERIMENT_PACKAGE>
<EXPERIMENT accession="SRX1"/>
<SAMPLE accession="SRS1"><SAMPLE_ATTRIBUTES><SAMPLE_ATTRIBUTE><TAG>k</TAG><VALUE>v</VALUE></SAMPLE_ATTRIBUTE></SAMPLE_ATTRIBUTES></SAMPLE>
<RUN_SET>
<RUN accession="SRR1" published="2024-01-02 03:04:05">
<SRAFiles>
<SRAFile cluster="public" filename="SRR1" url="https://sra-pub.s3.amazonaws.com/SRR1" semantic_name="run">
<Alternatives url="https://sra-download.ncbi.nlm.nih.gov/traces/sra/SRR1" org="NCBI"/>
<Alternatives url="s3://sra-pub-run-odp/sra/SRR1" org="AWS"/>
<Alternatives url="gs://sra-pub-run-1/SRR1" org="GCP"/>
</SRAFile>
</SRAFiles>
</RUN>
</RUN_SET>
</EXPERIMENT_PACKAGE>
</EXPERIMENT_PACKAGE_SET>"#;
        let pkgs = parse(XML).unwrap();
        let p = &pkgs["SRX1"];
        let urls = &p.run_urls["SRR1"];
        assert_eq!(urls.ncbi_sra.as_deref(), Some("https://sra-download.ncbi.nlm.nih.gov/traces/sra/SRR1"));
        assert_eq!(urls.s3.as_deref(), Some("s3://sra-pub-run-odp/sra/SRR1"));
        assert_eq!(urls.gs.as_deref(), Some("gs://sra-pub-run-1/SRR1"));
        assert_eq!(p.run_published.get("SRR1").map(String::as_str), Some("2024-01-02 03:04:05"));
    }

    #[test]
    fn real_srp174132_fixture_has_urls() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_xml_SRP174132.xml"),
        )
        .expect("fixture missing");
        let pkgs = parse(&body).unwrap();
        let mut runs_with_any_url = 0;
        for pkg in pkgs.values() {
            for urls in pkg.run_urls.values() {
                if urls.ncbi_sra.is_some() || urls.s3.is_some() || urls.gs.is_some() {
                    runs_with_any_url += 1;
                }
            }
        }
        assert!(runs_with_any_url > 0, "at least one run should have a download URL");
    }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p sradb-core --lib parse::experiment_package`
Expected: 4 tests PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/sradb-core/src/parse/experiment_package.rs
git commit -m "feat(parse): SRAFile alternatives (NCBI/AWS/GCP URLs) + RUN published"
```

---

## Task 6: Parse ENA filereport TSV

**Files:**
- Create: `crates/sradb-core/src/parse/ena_filereport.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs`

- [ ] **Step 1: Add module declaration**

```rust
//! Parsers for NCBI / ENA response payloads.

pub mod ena_filereport;
pub mod esummary;
pub mod experiment_package;
pub mod exp_xml;
pub mod runinfo;
pub mod sample_attrs;
```

- [ ] **Step 2: Implement parser**

Create `crates/sradb-core/src/parse/ena_filereport.rs`:

```rust
//! Parser for ENA filereport TSV (`/portal/api/filereport`).
//!
//! Each row maps a run accession to its fastq URLs (FTP and aspera) plus
//! md5/byte sizes. For paired-end runs, the `fastq_ftp` field holds two
//! `;`-separated paths; we split into per-mate vectors.

use crate::error::{Result, SradbError};

const CONTEXT: &str = "ena_filereport";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnaFilereportRow {
    pub run_accession: String,
    pub fastq_ftp: Vec<String>,    // 0..2 entries
    pub fastq_md5: Vec<String>,
    pub fastq_bytes: Vec<u64>,
    pub fastq_aspera: Vec<String>,
}

/// Parse an ENA filereport TSV body. Empty body → empty vec.
pub fn parse(body: &str) -> Result<Vec<EnaFilereportRow>> {
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .delimiter(b'\t')
        .flexible(true)
        .from_reader(body.as_bytes());

    let headers = reader.headers().map_err(|e| SradbError::Csv { context: CONTEXT, source: e })?
        .clone();
    let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let i_run = col("run_accession");
    let i_ftp = col("fastq_ftp");
    let i_md5 = col("fastq_md5");
    let i_bytes = col("fastq_bytes");
    let i_aspera = col("fastq_aspera");

    let mut out = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| SradbError::Csv { context: CONTEXT, source: e })?;
        let mut row = EnaFilereportRow::default();
        if let Some(i) = i_run {
            row.run_accession = record.get(i).unwrap_or_default().to_owned();
        }
        if row.run_accession.is_empty() {
            continue;
        }
        row.fastq_ftp = split_semi(record.get(i_ftp.unwrap_or(usize::MAX)));
        row.fastq_md5 = split_semi(record.get(i_md5.unwrap_or(usize::MAX)));
        row.fastq_bytes = split_semi(record.get(i_bytes.unwrap_or(usize::MAX)))
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        row.fastq_aspera = split_semi(record.get(i_aspera.unwrap_or(usize::MAX)));
        out.push(row);
    }
    Ok(out)
}

fn split_semi(s: Option<&str>) -> Vec<String> {
    s.unwrap_or("")
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "run_accession\tfastq_ftp\tfastq_md5\tfastq_bytes\tfastq_aspera\n\
SRR8361601\tftp.sra.ebi.ac.uk/vol1/fastq/SRR836/001/SRR8361601/SRR8361601_1.fastq.gz;ftp.sra.ebi.ac.uk/vol1/fastq/SRR836/001/SRR8361601/SRR8361601_2.fastq.gz\tabc;def\t1234567;7654321\tera-fasp@x;era-fasp@y\n";

    #[test]
    fn parses_paired_end() {
        let rows = parse(SAMPLE).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.run_accession, "SRR8361601");
        assert_eq!(r.fastq_ftp.len(), 2);
        assert!(r.fastq_ftp[0].ends_with("_1.fastq.gz"));
        assert!(r.fastq_ftp[1].ends_with("_2.fastq.gz"));
        assert_eq!(r.fastq_md5, vec!["abc".to_string(), "def".into()]);
        assert_eq!(r.fastq_bytes, vec![1234567, 7654321]);
    }

    #[test]
    fn parses_real_srr8361601_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ena/filereport_SRR8361601.tsv"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-ena-filereport SRR8361601` first");
        let rows = parse(&body).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.run_accession, "SRR8361601");
        assert!(!r.fastq_ftp.is_empty(), "should have at least one fastq URL");
    }

    #[test]
    fn empty_body_yields_empty_vec() {
        assert!(parse("").unwrap().is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib parse::ena_filereport`
Expected: 3 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/ena_filereport.rs
git commit -m "feat(parse): ENA filereport TSV parser"
```

---

## Task 7: ncbi/efetch async wrapper

**Files:**
- Modify: `crates/sradb-core/src/ncbi/mod.rs`
- Create: `crates/sradb-core/src/ncbi/efetch.rs`

- [ ] **Step 1: Add module declaration**

Read `crates/sradb-core/src/ncbi/mod.rs`. Add `pub mod efetch;`:

```rust
//! Wrappers for NCBI eUtils endpoints.

pub mod efetch;
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

- [ ] **Step 2: Implement efetch.rs**

Create `crates/sradb-core/src/ncbi/efetch.rs`:

```rust
//! NCBI efetch wrappers.

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT_RUNINFO: &str = "efetch_runinfo";
const CONTEXT_XML: &str = "efetch_xml";

/// Fetch one page of efetch retmode=runinfo (CSV) using a (`WebEnv`, `query_key`) handle.
#[allow(clippy::too_many_arguments)]
pub async fn efetch_runinfo_with_history(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    webenv: &str,
    query_key: &str,
    retstart: u32,
    retmax: u32,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/efetch.fcgi");
    let retstart_s = retstart.to_string();
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", &retstart_s),
        ("retmax", &retmax_s),
        ("rettype", "runinfo"),
        ("retmode", "csv"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT_RUNINFO, Service::Ncbi, &url, &q).await
}

/// Fetch one page of efetch retmode=xml (full EXPERIMENT_PACKAGE_SET) using a (`WebEnv`, `query_key`) handle.
#[allow(clippy::too_many_arguments)]
pub async fn efetch_full_xml_with_history(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    webenv: &str,
    query_key: &str,
    retstart: u32,
    retmax: u32,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/efetch.fcgi");
    let retstart_s = retstart.to_string();
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", &retstart_s),
        ("retmax", &retmax_s),
        ("rettype", "full"),
        ("retmode", "xml"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT_XML, Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn runinfo_calls_efetch_with_csv_args() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/efetch.fcgi"))
            .and(query_param("rettype", "runinfo"))
            .and(query_param("retmode", "csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Run,bases\nSRR1,100\n"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = efetch_runinfo_with_history(&http, &server.uri(), "sra", "WE", "QK", 0, 500, None)
            .await.unwrap();
        assert!(body.contains("Run,bases"));
    }

    #[tokio::test]
    async fn full_xml_calls_efetch_with_xml_args() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/efetch.fcgi"))
            .and(query_param("rettype", "full"))
            .and(query_param("retmode", "xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<EXPERIMENT_PACKAGE_SET/>"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = efetch_full_xml_with_history(&http, &server.uri(), "sra", "WE", "QK", 0, 500, None)
            .await.unwrap();
        assert_eq!(body, "<EXPERIMENT_PACKAGE_SET/>");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib ncbi::efetch`
Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/ncbi/mod.rs crates/sradb-core/src/ncbi/efetch.rs
git commit -m "feat(ncbi): efetch wrappers for runinfo CSV and full XML"
```

---

## Task 8: ENA filereport client

**Files:**
- Create: `crates/sradb-core/src/ena.rs`
- Modify: `crates/sradb-core/src/lib.rs` (add `pub mod ena;`)

- [ ] **Step 1: Add module to lib.rs**

Read `crates/sradb-core/src/lib.rs`. Add `pub mod ena;` to the module declarations:

```rust
pub mod accession;
pub mod client;
pub mod ena;
pub mod error;
pub mod http;
pub mod metadata;
pub mod model;
pub mod ncbi;
pub mod parse;
```

- [ ] **Step 2: Implement ena.rs**

Create `crates/sradb-core/src/ena.rs`:

```rust
//! ENA portal API client (filereport endpoint).

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT: &str = "ena_filereport";

/// Fetch the ENA filereport TSV for one run accession.
///
/// Returns the raw TSV body. Use `parse::ena_filereport::parse` to decode.
pub async fn fetch_filereport(
    http: &HttpClient,
    base_url: &str,
    run_accession: &str,
) -> Result<String> {
    let url = format!("{base_url}/portal/api/filereport");
    http.get_text(
        CONTEXT,
        Service::Ena,
        &url,
        &[
            ("accession", run_accession),
            ("result", "read_run"),
            ("fields", "fastq_ftp,fastq_md5,fastq_bytes,fastq_aspera"),
            ("format", "tsv"),
        ],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn filereport_returns_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/portal/api/filereport"))
            .and(query_param("accession", "SRR1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("run_accession\tfastq_ftp\nSRR1\tx.fastq.gz"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = fetch_filereport(&http, &server.uri(), "SRR1").await.unwrap();
        assert!(body.contains("SRR1"));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib ena`
Expected: 1 test PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/ena.rs
git commit -m "feat(ena): filereport client"
```

---

## Task 9: Detailed orchestrator branch

**Files:**
- Modify: `crates/sradb-core/src/metadata.rs`

This task wires runinfo + experiment_package + ENA fan-out into the orchestrator, gated by `opts.detailed`.

- [ ] **Step 1: Read current metadata.rs**

The current orchestrator chains esearch → esummary → parse → assemble. Detailed mode adds three more steps after the rows are assembled but before they're returned.

- [ ] **Step 2: Replace metadata.rs**

Overwrite `crates/sradb-core/src/metadata.rs`:

```rust
//! Metadata orchestrator: chains esearch → esummary → parse → typed `MetadataRow`.
//!
//! When `opts.detailed = true`, additional fetches augment each row with:
//! - runinfo CSV (refines `total_bases`, `total_size`, `published`)
//! - EXPERIMENT_PACKAGE_SET XML (sample attributes, NCBI/S3/GS download URLs)
//! - ENA filereport per run (fastq URLs, fan-out concurrency = 8)

use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::model::{MetadataOpts, MetadataRow, Run, RunUrls};
use crate::ncbi::{efetch, esearch, esummary};
use crate::{ena, parse};

const ENA_CONCURRENCY: usize = 8;

/// Drive the full metadata flow for a single accession.
pub async fn fetch_metadata(
    http: &HttpClient,
    ncbi_base_url: &str,
    ena_base_url: &str,
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

    // Default-mode rows: assemble from esummary first.
    let mut rows: Vec<MetadataRow> = Vec::with_capacity(result.count as usize);
    let mut retstart: u32 = 0;
    let total = u32::try_from(result.count).unwrap_or(u32::MAX);
    while retstart < total {
        let body = esummary::esummary_with_history(
            http, ncbi_base_url, "sra", &result.webenv, &result.query_key,
            retstart, page, api_key,
        ).await?;
        let docs = parse::esummary::parse(&body)?;
        if docs.is_empty() { break; }
        for d in docs {
            rows.extend(assemble_rows(d)?);
        }
        retstart += page;
    }

    if !opts.detailed {
        return Ok(rows);
    }

    // Detailed-mode augmentation.
    augment_with_runinfo(http, ncbi_base_url, &result.webenv, &result.query_key, api_key, page, total, &mut rows).await?;
    augment_with_experiment_package(http, ncbi_base_url, &result.webenv, &result.query_key, api_key, page, total, &mut rows).await?;
    augment_with_ena_fastq(http, ena_base_url, &mut rows).await?;
    Ok(rows)
}

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
    let rows = runs.into_iter().map(|raw_run| MetadataRow {
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
    }).collect();
    Ok(rows)
}

async fn augment_with_runinfo(
    http: &HttpClient,
    ncbi_base_url: &str,
    webenv: &str,
    query_key: &str,
    api_key: Option<&str>,
    page: u32,
    total: u32,
    rows: &mut [MetadataRow],
) -> Result<()> {
    let mut runinfo: std::collections::HashMap<String, parse::runinfo::RunInfo> = std::collections::HashMap::new();
    let mut retstart: u32 = 0;
    while retstart < total {
        let body = efetch::efetch_runinfo_with_history(
            http, ncbi_base_url, "sra", webenv, query_key, retstart, page, api_key,
        ).await?;
        let map = parse::runinfo::parse(&body)?;
        runinfo.extend(map);
        retstart += page;
    }
    for row in rows.iter_mut() {
        if let Some(info) = runinfo.get(&row.run.accession) {
            if let Some(b) = info.bases { row.run.total_bases = Some(b); }
            if let Some(mb) = info.size_mb {
                // size_MB is in megabytes; the public field is bytes.
                row.run.total_size = Some(mb.saturating_mul(1_000_000));
            }
            if let Some(d) = &info.release_date {
                row.run.published = Some(d.clone());
            }
        }
    }
    Ok(())
}

async fn augment_with_experiment_package(
    http: &HttpClient,
    ncbi_base_url: &str,
    webenv: &str,
    query_key: &str,
    api_key: Option<&str>,
    page: u32,
    total: u32,
    rows: &mut [MetadataRow],
) -> Result<()> {
    let mut packages: std::collections::HashMap<String, parse::experiment_package::ExperimentPackage> =
        std::collections::HashMap::new();
    let mut retstart: u32 = 0;
    while retstart < total {
        let body = efetch::efetch_full_xml_with_history(
            http, ncbi_base_url, "sra", webenv, query_key, retstart, page, api_key,
        ).await?;
        let map = parse::experiment_package::parse(&body)?;
        packages.extend(map);
        retstart += page;
    }
    for row in rows.iter_mut() {
        if let Some(pkg) = packages.get(&row.experiment.accession) {
            // Sample attributes: convert per-experiment attrs into the row's sample.
            row.sample.attributes = pkg.sample_attributes.clone();
            // Per-run download URLs.
            if let Some(urls) = pkg.run_urls.get(&row.run.accession) {
                row.run.urls.ncbi_sra = urls.ncbi_sra.clone();
                row.run.urls.s3 = urls.s3.clone();
                row.run.urls.gs = urls.gs.clone();
            }
            // Run published (overrides default-mode fallback).
            if let Some(p) = pkg.run_published.get(&row.run.accession) {
                row.run.published = Some(p.clone());
            }
        }
    }
    Ok(())
}

async fn augment_with_ena_fastq(
    http: &HttpClient,
    ena_base_url: &str,
    rows: &mut [MetadataRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let semaphore = Arc::new(Semaphore::new(ENA_CONCURRENCY));
    let http_owned = http.clone();
    let base = ena_base_url.to_owned();
    let mut futures = FuturesUnordered::new();
    for (idx, row) in rows.iter().enumerate() {
        let semaphore = semaphore.clone();
        let http = http_owned.clone();
        let base = base.clone();
        let acc = row.run.accession.clone();
        futures.push(async move {
            let _permit = semaphore.acquire().await.expect("semaphore not closed");
            let body = ena::fetch_filereport(&http, &base, &acc).await;
            (idx, body)
        });
    }
    while let Some((idx, body)) = futures.next().await {
        let body = match body {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("ENA filereport failed for {}: {e}", rows[idx].run.accession);
                continue;
            }
        };
        let parsed = match parse::ena_filereport::parse(&body) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("ENA filereport parse failed for {}: {e}", rows[idx].run.accession);
                continue;
            }
        };
        if let Some(r) = parsed.into_iter().find(|r| r.run_accession == rows[idx].run.accession) {
            rows[idx].run.urls.ena_fastq_ftp = r.fastq_ftp.iter().map(|p| {
                if p.starts_with("ftp://") || p.starts_with("http") {
                    p.clone()
                } else {
                    format!("ftp://{p}")
                }
            }).collect();
            rows[idx].run.urls.ena_fastq_http = r.fastq_ftp.iter().map(|p| {
                if p.starts_with("http") {
                    p.clone()
                } else {
                    let trimmed = p.strip_prefix("ftp://").unwrap_or(p);
                    format!("https://{trimmed}")
                }
            }).collect();
        }
    }
    Ok(())
}
```

The signature of `fetch_metadata` changed: it now takes `ena_base_url` between `ncbi_base_url` and `api_key`. Task 10 updates the `SraClient::metadata` caller.

- [ ] **Step 3: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: a compile error in `client.rs` because `fetch_metadata`'s signature changed. That's expected — Task 10 fixes it.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/metadata.rs
git commit -m "feat(metadata): detailed-mode orchestrator with runinfo+exp_pkg+ENA fan-out"
```

The workspace is briefly broken at this commit; Task 10 makes it green again.

---

## Task 10: Wire ena_base_url through SraClient::metadata

**Files:**
- Modify: `crates/sradb-core/src/client.rs`

- [ ] **Step 1: Read current client.rs**

Find the `metadata` method:

```rust
pub async fn metadata(&self, accession: &str, opts: &crate::model::MetadataOpts) -> Result<Vec<crate::model::MetadataRow>> {
    crate::metadata::fetch_metadata(
        &self.http,
        &self.cfg.ncbi_base_url,
        self.cfg.api_key.as_deref(),
        accession,
        opts,
    ).await
}
```

- [ ] **Step 2: Add the new parameter**

Replace the body to pass `ena_base_url`:

```rust
pub async fn metadata(&self, accession: &str, opts: &crate::model::MetadataOpts) -> Result<Vec<crate::model::MetadataRow>> {
    crate::metadata::fetch_metadata(
        &self.http,
        &self.cfg.ncbi_base_url,
        &self.cfg.ena_base_url,
        self.cfg.api_key.as_deref(),
        accession,
        opts,
    ).await
}
```

- [ ] **Step 3: Build + test**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: previous test count maintained (28 from slice 2 + new tests from Tasks 1-8 in this slice).

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/client.rs
git commit -m "feat(client): pass ena_base_url through to metadata orchestrator"
```

---

## Task 11: CLI --detailed flag

**Files:**
- Modify: `crates/sradb-cli/src/cmd/metadata.rs`

- [ ] **Step 1: Read current handler**

The `MetadataArgs` struct currently has `accessions`, `format`, `page_size`. The `run()` handler builds `MetadataOpts { detailed: false, ... }` unconditionally.

- [ ] **Step 2: Add --detailed to MetadataArgs**

Insert into the struct (between `format` and `page_size`):

```rust
    /// Fetch detailed metadata: sample attributes, NCBI/S3/GS download URLs,
    /// ENA fastq URLs.
    #[arg(long, default_value_t = false)]
    pub detailed: bool,
```

- [ ] **Step 3: Plumb the flag into MetadataOpts**

Replace the opts construction in `run()`:

```rust
    let opts = MetadataOpts {
        detailed: args.detailed,
        enrich: false,
        page_size: args.page_size,
    };
```

- [ ] **Step 4: Build**

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 5: Smoke help**

Run: `cargo run -p sradb-cli --quiet -- metadata --help 2>&1 | tail -15`
Expected: includes `--detailed` flag.

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-cli/src/cmd/metadata.rs
git commit -m "feat(cli): --detailed flag on sradb metadata"
```

---

## Task 12: Detailed output columns + dynamic sample-attribute columns

**Files:**
- Modify: `crates/sradb-cli/src/output.rs`

The detailed output has fixed columns (ENA URLs, NCBI/S3/GS URLs) plus dynamic sample-attribute columns (one per unique tag across all rows). Tag names get a `sample_attribute_` prefix.

- [ ] **Step 1: Refactor `write` to take a `detailed: bool` flag**

The current signature:
```rust
pub fn write(rows: &[MetadataRow], format: Format, mut out: impl Write) -> io::Result<()>
```

Change to:
```rust
pub fn write(rows: &[MetadataRow], format: Format, detailed: bool, mut out: impl Write) -> io::Result<()>
```

- [ ] **Step 2: Add a function that computes columns**

Add this function at module level (before `write`):

```rust
const DETAILED_FIXED_COLUMNS: &[&str] = &[
    "ena_fastq_http_1",
    "ena_fastq_http_2",
    "ena_fastq_ftp_1",
    "ena_fastq_ftp_2",
    "ncbi_url",
    "s3_url",
    "gs_url",
];

fn compute_columns(rows: &[MetadataRow], detailed: bool) -> Vec<String> {
    let mut cols: Vec<String> = TSV_COLUMNS.iter().map(|s| (*s).to_owned()).collect();
    if detailed {
        cols.extend(DETAILED_FIXED_COLUMNS.iter().map(|s| (*s).to_owned()));
        // Dynamic sample-attribute columns: union of keys across all rows, sorted.
        let mut keys: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for row in rows {
            for k in row.sample.attributes.keys() {
                keys.insert(format!("sample_attribute_{k}"));
            }
        }
        cols.extend(keys);
    }
    cols
}
```

- [ ] **Step 3: Extend the `cell` function with detailed columns**

Append match arms before the `_ => String::new()` catch-all:

```rust
        "ena_fastq_http_1" => row.run.urls.ena_fastq_http.first().cloned().unwrap_or_default(),
        "ena_fastq_http_2" => row.run.urls.ena_fastq_http.get(1).cloned().unwrap_or_default(),
        "ena_fastq_ftp_1" => row.run.urls.ena_fastq_ftp.first().cloned().unwrap_or_default(),
        "ena_fastq_ftp_2" => row.run.urls.ena_fastq_ftp.get(1).cloned().unwrap_or_default(),
        "ncbi_url" => opt_string(&row.run.urls.ncbi_sra),
        "s3_url" => opt_string(&row.run.urls.s3),
        "gs_url" => opt_string(&row.run.urls.gs),
        col if col.starts_with("sample_attribute_") => {
            let key = &col["sample_attribute_".len()..];
            row.sample.attributes.get(key).cloned().unwrap_or_default()
        }
```

- [ ] **Step 4: Update write_tsv to use computed columns**

Change `write_tsv` body:

```rust
fn write_tsv<W: Write>(rows: &[MetadataRow], detailed: bool, out: &mut W) -> io::Result<()> {
    let columns = compute_columns(rows, detailed);
    writeln!(out, "{}", columns.join("\t"))?;
    for row in rows {
        let cells: Vec<String> = columns.iter().map(|c| sanitize_tsv(&cell(row, c))).collect();
        writeln!(out, "{}", cells.join("\t"))?;
    }
    Ok(())
}
```

And update `write`:

```rust
pub fn write(rows: &[MetadataRow], format: Format, detailed: bool, mut out: impl Write) -> io::Result<()> {
    match format {
        Format::Tsv => write_tsv(rows, detailed, &mut out),
        Format::Json => write_json(rows, &mut out),
        Format::Ndjson => write_ndjson(rows, &mut out),
    }
}
```

JSON/NDJSON don't need a `detailed` switch — they always serialize the full struct (which includes `urls` and `attributes` fields), populated or empty.

- [ ] **Step 5: Update the existing tests**

The existing 3 tests call `write(rows, format, &mut out)`. They need a new `detailed: bool` argument:

```rust
write(std::slice::from_ref(&fixture_row()), Format::Tsv, false, &mut out).unwrap();
```

(Apply to all three test calls.)

- [ ] **Step 6: Add a detailed TSV test**

Append inside `mod tests`:

```rust
    #[test]
    fn tsv_detailed_includes_urls_and_attrs() {
        let mut row = fixture_row();
        row.run.urls.ena_fastq_http = vec!["https://x_1.fastq.gz".into(), "https://x_2.fastq.gz".into()];
        row.run.urls.ncbi_sra = Some("https://sra-download/SRR1".into());
        row.sample.attributes.insert("source_name".into(), "liver".into());
        row.sample.attributes.insert("cell type".into(), "hepatocyte".into());

        let mut out = Vec::new();
        write(std::slice::from_ref(&row), Format::Tsv, true, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("ena_fastq_http_1"));
        assert!(text.contains("ena_fastq_http_2"));
        assert!(text.contains("ncbi_url"));
        assert!(text.contains("sample_attribute_source_name"));
        assert!(text.contains("sample_attribute_cell type"));
        assert!(text.contains("https://x_1.fastq.gz"));
        assert!(text.contains("liver"));
        assert!(text.contains("hepatocyte"));
    }
```

- [ ] **Step 7: Update cmd/metadata.rs to pass the detailed flag to output::write**

In `crates/sradb-cli/src/cmd/metadata.rs`, find:

```rust
    output::write(&all_rows, args.format, &mut handle).map_err(anyhow::Error::from)?;
```

Replace with:

```rust
    output::write(&all_rows, args.format, args.detailed, &mut handle).map_err(anyhow::Error::from)?;
```

- [ ] **Step 8: Run tests**

Run: `cargo test -p sradb-cli --bin sradb`
Expected: 4 tests PASS (3 existing + 1 new).

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add crates/sradb-cli/src/output.rs crates/sradb-cli/src/cmd/metadata.rs
git commit -m "feat(cli): detailed columns + dynamic sample-attribute columns"
```

---

## Task 13: End-to-end detailed test

**Files:**
- Create: `crates/sradb-core/tests/metadata_detailed_e2e.rs`

- [ ] **Step 1: Write the test**

Create `crates/sradb-core/tests/metadata_detailed_e2e.rs`:

```rust
//! End-to-end test of the detailed metadata path against captured fixtures.

use sradb_core::{ClientConfig, MetadataOpts, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn metadata_detailed_srp174132() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).expect("esearch fixture");
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).expect("esummary fixture");
    let runinfo_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/efetch_runinfo_SRP174132.csv")).expect("runinfo fixture");
    let xml_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/efetch_xml_SRP174132.xml")).expect("efetch xml fixture");
    let ena_body = std::fs::read_to_string(workspace.join("tests/data/ena/filereport_SRR8361601.tsv")).expect("ena fixture");

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
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .and(query_param("rettype", "runinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_string(runinfo_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .and(query_param("rettype", "full"))
        .respond_with(ResponseTemplate::new(200).set_body_string(xml_body))
        .mount(&server)
        .await;
    // ENA: only one fixture, but the orchestrator fans out per-run.
    // For runs we have no fixture, return an empty body (parser yields empty rows).
    Mock::given(method("GET"))
        .and(path("/portal/api/filereport"))
        .and(query_param("accession", "SRR8361601"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ena_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portal/api/filereport"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let opts = MetadataOpts { detailed: true, enrich: false, page_size: 500 };
    let mut rows = client.metadata("SRP174132", &opts).await.unwrap();
    rows.sort_by(|a, b| a.run.accession.cmp(&b.run.accession));

    assert!(!rows.is_empty(), "expected ≥ 1 row");

    // Sample attributes populated for every row (from EXPERIMENT_PACKAGE_SET).
    for r in &rows {
        assert!(!r.sample.attributes.is_empty(), "{} should have sample attrs", r.run.accession);
    }

    // The single ENA-fixture run should have fastq URLs.
    let r = rows.iter().find(|r| r.run.accession == "SRR8361601").expect("SRR8361601 must be present");
    assert!(!r.run.urls.ena_fastq_ftp.is_empty(), "SRR8361601 should have ENA fastq FTP URLs");
    assert!(!r.run.urls.ena_fastq_http.is_empty(), "SRR8361601 should have ENA fastq HTTP URLs");

    // Some run should have at least one of NCBI/S3/GS URLs from the EXPERIMENT_PACKAGE_SET.
    let any_dl = rows.iter().any(|r| {
        r.run.urls.ncbi_sra.is_some() || r.run.urls.s3.is_some() || r.run.urls.gs.is_some()
    });
    assert!(any_dl, "at least one run should have a download URL from SRAFiles");
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sradb-core --test metadata_detailed_e2e 2>&1 | tail -10`
Expected: 1 test PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/metadata_detailed_e2e.rs
git commit -m "test(core): wiremock e2e for --detailed metadata path"
```

---

## Task 14: Live smoke test against real APIs

**Files:** none changed; verification only.

- [ ] **Step 1: Build release-debug**

Run: `cargo build --release -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 2: Live smoke**

Run: `cargo run --release -p sradb-cli --quiet -- metadata SRP174132 --detailed --format json 2>&1 | head -50`

Expected: JSON array; first row has nonempty `sample.attributes`, `run.urls.ena_fastq_http` (or `ena_fastq_ftp`), and at least one of `run.urls.ncbi_sra` / `run.urls.s3` / `run.urls.gs`.

If network is blocked, the wiremock e2e from Task 13 already proves correctness — DONE_WITH_CONCERNS is acceptable here.

- [ ] **Step 3: Live smoke — TSV detailed**

Run: `cargo run --release -p sradb-cli --quiet -- metadata SRP174132 --detailed --format tsv 2>&1 | head -3 | tr '\t' '|' | head -3`

Expected: header line (very wide — fixed + dynamic columns concatenated with `|`), then data rows.

---

## Task 15: Final verification

**Files:** none changed; verification only.

- [ ] **Step 1: Build**

Run: `cargo build --workspace --all-targets 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 2: Tests**

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: PASS, total ≥ 40 (28 from slice 2 + ~14 new from this slice).

- [ ] **Step 3: Clippy**

Run: `RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 4: Fmt**

Run: `cargo fmt --all -- --check 2>&1 | tail -2`
Expected: PASS. If fail: `cargo fmt --all` and commit separately.

- [ ] **Step 5: Mark plan complete**

Edit this plan file and add `✅` to each completed task heading.

```bash
git add docs/superpowers/plans/2026-04-26-sradb-rs-slice-3-detailed.md
git commit -m "docs(plan): mark slice-3 tasks complete"
```

- [ ] **Step 6: Tag**

```bash
git tag -a slice-3-detailed -m "Slice 3: --detailed metadata with sample attrs, SRA download URLs, ENA fastq URLs"
```

---

## What this slice does NOT include (intentional deferrals)

- `study_geo_accession` / `experiment_geo_accession` (needs convert engine — slice 4).
- PMIDs (also needs convert engine — slice 4).
- ENA fan-out timeout/retry tuning beyond the existing HttpClient defaults.
- Aspera URLs in TSV output (parsed but not exposed; the column would clutter the default detailed view).

## Definition of done for slice 3

1. `cargo build --workspace` clean.
2. `cargo test --workspace` clean — ≥40 tests.
3. `cargo clippy -- -Dwarnings` clean.
4. `cargo fmt --check` clean.
5. `sradb metadata SRP174132 --detailed --format json` against live NCBI shows non-empty `sample.attributes`, `run.urls.ena_fastq_http`, and at least one of `ncbi_sra`/`s3`/`gs` per row.
6. Detailed wiremock e2e green.
7. `git tag slice-3-detailed` created.
