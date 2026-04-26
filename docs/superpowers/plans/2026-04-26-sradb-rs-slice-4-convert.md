# sradb-rs Slice 4: Accession Conversion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land `sradb convert <FROM> <TO> <ACCESSION>...` covering all 25+ pysradb conversion methods through one engine + strategy lookup table. Most conversions reuse the existing metadata orchestrator and project a field; GSE-related conversions go through NCBI's `db=gds` esummary.

**Architecture:** A small `convert.rs` module with a `Strategy` enum and a static `(FromKind, ToKind) → Strategy` lookup. Two strategies: `ProjectFromMetadata` (reuses `metadata::fetch_metadata`, projects one field per row) and `GdsLookup` (db=gds esummary, parses `extrelations`). `SraClient::convert(...)` and `convert_detailed(...)` are thin facade methods.

**Tech Stack:** existing `quick-xml`, the slice-2 metadata orchestrator, `serde_json` for the JSON response form of `db=gds` esummary.

**Reference:** Spec at `docs/superpowers/specs/2026-04-25-sradb-rs-design.md`. Slices 1-3: tags `slice-1-foundation`, `slice-2-metadata`, `slice-3-detailed`.

---

## Background

**pysradb conversion table** (audit reference):

| from \ to | SRP | SRX | SRR | SRS | GSE | GSM |
| --- | --- | --- | --- | --- | --- | --- |
| **SRP** | — | ✓ | ✓ | ✓ | ✓ | — |
| **SRX** | ✓ | — | ✓ | ✓ | — | ✓ |
| **SRR** | ✓ | ✓ | — | ✓ | — | ✓ |
| **SRS** | — | ✓ | — | — | — | ✓ |
| **GSE** | ✓ | — | — | — | — | ✓ |
| **GSM** | ✓ | ✓ | ✓ | ✓ | ✓ | — |

We implement every cell with a check-mark above. The diagonal (`SRP→SRP` etc.) is allowed and returns the input unchanged.

### NCBI db=gds (GEO Datasets) wire shape

For GSE↔SRP and GSM↔GSE, we use NCBI's `db=gds` esummary. The eUtils JSON format here returns one record per GDS UID:

```
GET {ncbi}/esummary.fcgi?db=gds&id=200056924&retmode=json
```

```json
{
  "header": { ... },
  "result": {
    "uids": ["200056924"],
    "200056924": {
      "uid": "200056924",
      "accession": "GSE56924",
      "gse": "56924",
      "entrytype": "GSE",
      "n_samples": 96,
      "samples": [{"accession": "GSM1371490", "title": "..."}],
      "extrelations": [{"relationtype": "SRA", "targetobject": "SRP041298", "targetftplink": "..."}]
    }
  }
}
```

Key fields:
- `accession` and `entrytype` ("GSE" / "GSM" / "GPL")
- `extrelations[].targetobject` → SRP accession (when entrytype is GSE)
- `samples[].accession` → child GSMs (when entrytype is GSE)

To go from a string accession to a UID, we use `esearch` against `db=gds`:

```
GET {ncbi}/esearch.fcgi?db=gds&term=GSE56924&retmode=json
```

The JSON shape matches the existing `esearch` parser (count, idlist, webenv, querykey).

### Conversion strategies

Two strategies cover all cases:

**`ProjectFromMetadata { project: ProjField }`** — call `fetch_metadata(input)` (which uses `db=sra`), project one field per row, dedupe.

```rust
enum ProjField { StudyAccession, ExperimentAccession, RunAccession, SampleAccession, GeoExperimentFromTitle }
```

`db=sra` esearch accepts SRP/SRX/SRR/SRS/GSM as search terms (NCBI cross-references work). Returns SRA records linking to all related accessions in the row.

The `GeoExperimentFromTitle` projector parses GSM out of the experiment title (pysradb does the same — titles look like `"GSM3526037: RNA-Seq ..."`).

**`GdsLookup { project: GdsField }`** — call `gds_esearch + gds_esummary(input)`, parse the JSON response, project one field.

```rust
enum GdsField { GseAccession, SrpFromExtrelations, GsmsFromSamples, GseFromGsmExtrelations }
```

### Strategy table

```rust
match (from, to) {
    (Srp, Srx | Srr | Srs) => ProjectFromMetadata { project: <SRX/SRR/SRS> },
    (Srx, Srp | Srr | Srs) => ProjectFromMetadata { project: <SRP/SRR/SRS> },
    (Srr, Srp | Srx | Srs) => ProjectFromMetadata { project: <SRP/SRX/SRS> },
    (Srs, Srx)             => ProjectFromMetadata { project: SRX },
    (Gsm, Srp | Srx | Srr | Srs) => ProjectFromMetadata { project: <SRP/SRX/SRR/SRS> },
    (Srx | Srr | Srs, Gsm) => ProjectFromMetadata { project: GeoExperimentFromTitle },

    (Srp, Gse) => GdsLookup { project: GseAccession },
    (Gsm, Gse) => GdsLookup { project: GseFromGsmExtrelations },
    (Gse, Srp) => GdsLookup { project: SrpFromExtrelations },
    (Gse, Gsm) => GdsLookup { project: GsmsFromSamples },

    (k, k2) if k == k2 => Identity,
    _                  => UnsupportedConversion,
}
```

Some pairs aren't in pysradb (e.g., `Gsm→Gsm`, `Gse→Srx`). For those, we either return `UnsupportedConversion` or chain (e.g., `Gse→Srx` = `Gse→Srp→Srx`). Slice 4 returns `UnsupportedConversion` for un-tabled pairs; chaining can be a polish task.

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/parse/gds_esummary.rs` | Parse db=gds esummary JSON into `GdsRecord` |
| `crates/sradb-core/src/ncbi/gds.rs` | Async wrapper for db=gds esearch + esummary |
| `crates/sradb-core/src/convert.rs` | `Strategy` enum, lookup table, dispatch |
| `crates/sradb-core/src/lib.rs` | (modify) Add `pub mod convert;` |
| `crates/sradb-core/src/client.rs` | (modify) Add `convert` and `convert_detailed` methods |
| `crates/sradb-cli/src/cmd.rs` | (modify) `pub mod convert;` |
| `crates/sradb-cli/src/cmd/convert.rs` | CLI handler |
| `crates/sradb-cli/src/main.rs` | (modify) register `Convert` subcommand |
| `tools/capture-fixtures/src/main.rs` | (modify) Add `save-gds-esearch` and `save-gds-esummary` subcommands |
| `tests/data/ncbi/gds_esearch_GSE56924.json` | Captured GDS esearch fixture |
| `tests/data/ncbi/gds_esummary_GSE56924.json` | Captured GDS esummary fixture |
| `tests/data/ncbi/gds_esummary_GSM1371490.json` | Captured GSM gds esummary fixture |
| `crates/sradb-core/tests/convert_e2e.rs` | Wiremock e2e covering both strategies |

---

## Task 1: Capture db=gds fixtures

**Files:**
- Modify: `tools/capture-fixtures/src/main.rs` (add 2 subcommands)
- Create: `tests/data/ncbi/gds_esearch_GSE56924.json`, `tests/data/ncbi/gds_esummary_GSE56924.json`, `tests/data/ncbi/gds_esummary_GSM1371490.json`

- [ ] **Step 1: Read current main.rs**

The capture-fixtures binary currently has 7 subcommands: `Info`, `Metadata`, `SaveEsearch`, `SaveEsummary`, `SaveEfetchRuninfo`, `SaveEfetchXml`, `SaveEnaFilereport`. We add two GDS variants.

- [ ] **Step 2: Add Cmd variants**

Inside the `Cmd` enum, after `SaveEnaFilereport`:

```rust
    /// Capture a db=gds esearch response and write it to
    /// `tests/data/ncbi/gds_esearch_<accession>.json`.
    SaveGdsEsearch {
        accession: String,
    },
    /// Capture a db=gds esummary response (uses esearch first to get UID) and write it to
    /// `tests/data/ncbi/gds_esummary_<accession>.json`.
    SaveGdsEsummary {
        accession: String,
    },
```

- [ ] **Step 3: Add match arms**

Inside the `match cli.cmd` block:

```rust
        Cmd::SaveGdsEsearch { accession } => save_gds_esearch(&accession).await,
        Cmd::SaveGdsEsummary { accession } => save_gds_esummary(&accession).await,
```

- [ ] **Step 4: Add helper functions**

Append to `tools/capture-fixtures/src/main.rs` (before `run_metadata_dump`):

```rust
async fn gds_esearch_raw(
    client: &HttpClient,
    cfg: &sradb_core::ClientConfig,
    accession: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/esearch.fcgi", cfg.ncbi_base_url);
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "gds"),
        ("term", accession),
        ("retmode", "json"),
        ("retmax", "20"),
    ];
    if let Some(ref k) = cfg.api_key {
        q.push(("api_key", k));
    }
    Ok(client.get_text("gds_esearch", Service::Ncbi, &url, &q).await?)
}

async fn gds_esummary_raw(
    client: &HttpClient,
    cfg: &sradb_core::ClientConfig,
    uid: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/esummary.fcgi", cfg.ncbi_base_url);
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "gds"),
        ("id", uid),
        ("retmode", "json"),
    ];
    if let Some(ref k) = cfg.api_key {
        q.push(("api_key", k));
    }
    Ok(client.get_text("gds_esummary", Service::Ncbi, &url, &q).await?)
}

async fn save_gds_esearch(accession: &str) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let body = gds_esearch_raw(&client, &cfg, accession).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("gds_esearch_{accession}.json"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}

async fn save_gds_esummary(accession: &str) -> anyhow::Result<()> {
    let cfg = sradb_core::ClientConfig::default();
    let client = make_client(&cfg)?;
    let esearch_body = gds_esearch_raw(&client, &cfg, accession).await?;
    let v: serde_json::Value = serde_json::from_str(&esearch_body)?;
    let uid = v["esearchresult"]["idlist"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|x| x.as_str())
        .ok_or_else(|| anyhow::anyhow!("no UID in gds esearch response for {accession}"))?
        .to_owned();
    let body = gds_esummary_raw(&client, &cfg, &uid).await?;
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("gds_esummary_{accession}.json"));
    std::fs::write(&path, body.as_bytes())?;
    println!("wrote {} ({} bytes)", path.display(), body.len());
    Ok(())
}
```

- [ ] **Step 5: Build**

Run: `cargo build -p capture-fixtures 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 6: Capture fixtures (live network required)**

Run, in order:
```bash
cargo run --quiet -p capture-fixtures -- save-gds-esearch GSE56924
cargo run --quiet -p capture-fixtures -- save-gds-esummary GSE56924
cargo run --quiet -p capture-fixtures -- save-gds-esummary GSM1371490
```

Expected outputs (sizes approximate):
- `gds_esearch_GSE56924.json` (~500-1000 bytes)
- `gds_esummary_GSE56924.json` (~30-100 KB; 96 child GSM samples)
- `gds_esummary_GSM1371490.json` (~2-5 KB; single GSM record with extrelations)

If the network is blocked, BLOCKED.

- [ ] **Step 7: Spot-check fixtures**

Run:
```bash
head -c 200 tests/data/ncbi/gds_esearch_GSE56924.json
python3 -c "import json; d=json.load(open('tests/data/ncbi/gds_esummary_GSE56924.json')); uid=d['result']['uids'][0]; r=d['result'][uid]; print('accession:', r.get('accession'), 'entrytype:', r.get('entrytype'), 'n_samples:', r.get('n_samples'), 'extrelations[0]:', r.get('extrelations',[None])[0])"
python3 -c "import json; d=json.load(open('tests/data/ncbi/gds_esummary_GSM1371490.json')); uid=d['result']['uids'][0]; r=d['result'][uid]; print('accession:', r.get('accession'), 'entrytype:', r.get('entrytype'), 'extrelations[0]:', r.get('extrelations',[None])[0])"
```

Expected:
- esearch starts with `{"header":{"type":"esearch"...}` and contains `"idlist":[...]`
- GSE56924 esummary: accession=GSE56924, entrytype=GSE, n_samples=96, extrelations contains an SRA targetobject (SRP041298)
- GSM1371490 esummary: accession=GSM1371490, entrytype=GSM, extrelations may also contain a targetobject (the parent SRP)

- [ ] **Step 8: Commit**

```bash
git add tools/capture-fixtures/src/main.rs tests/data/ncbi/gds_*.json
git commit -m "feat(tools): save-gds-esearch / save-gds-esummary; capture GSE56924 + GSM1371490 fixtures"
```

## Context for Task 1

Slice 4 starts here, branched from slice-3-detailed (HEAD: `4614656`). Working dir: `/home/xzg/project/sradb_rs/`. Branch: `slice-4-convert`.

NCBI's `db=gds` UIDs follow a different prefix scheme than SRA UIDs but the eUtils API is identical. The two-step pattern (esearch → idlist → esummary by UID) mirrors what we did with `db=sra` in slices 2-3.

If `gds_esummary_GSM1371490.json` doesn't have an `extrelations` array (older GSMs sometimes lack it), the GSM→GSE conversion test will need a different fixture — note this in your report and we'll pick a different GSM accession.

---

## Task 2: Parse db=gds esummary JSON

**Files:**
- Create: `crates/sradb-core/src/parse/gds_esummary.rs`
- Modify: `crates/sradb-core/src/parse/mod.rs`

- [ ] **Step 1: Update parse/mod.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/parse/mod.rs`. Add `pub mod gds_esummary;` (alphabetical, between `experiment_package` and `exp_xml`):

```rust
//! Parsers for NCBI / ENA response payloads.

pub mod ena_filereport;
pub mod esummary;
pub mod experiment_package;
pub mod exp_xml;
pub mod gds_esummary;
pub mod runinfo;
pub mod sample_attrs;
```

- [ ] **Step 2: Implement gds_esummary.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/parse/gds_esummary.rs`:

```rust
//! Parser for NCBI db=gds esummary JSON responses.
//!
//! The shape is `{"result": {"uids": [...], "<uid>": { ... }, ...}}`. We project
//! the fields needed for accession conversion: `accession`, `entrytype`,
//! `samples` (children for GSEs), and `extrelations` (cross-DB links).

use serde::Deserialize;

use crate::error::{Result, SradbError};

const CONTEXT: &str = "gds_esummary";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdsRecord {
    pub uid: String,
    pub accession: String,
    pub entry_type: String, // "GSE", "GSM", "GPL"
    pub n_samples: Option<u32>,
    pub samples: Vec<GdsSample>,
    pub extrelations: Vec<GdsExtRelation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GdsSample {
    #[serde(default)]
    pub accession: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GdsExtRelation {
    #[serde(default, rename = "relationtype")]
    pub relation_type: String, // typically "SRA"
    #[serde(default, rename = "targetobject")]
    pub target_object: String, // typically an SRP accession
}

/// Parse a db=gds esummary JSON body into one record per UID.
pub fn parse(body: &str) -> Result<Vec<GdsRecord>> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|source| SradbError::Json {
        context: CONTEXT,
        source,
    })?;
    let result = v.get("result").ok_or_else(|| SradbError::Parse {
        endpoint: CONTEXT,
        message: "missing `result` field".into(),
    })?;
    let uids = result
        .get("uids")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| SradbError::Parse {
            endpoint: CONTEXT,
            message: "missing `result.uids` array".into(),
        })?;

    let mut out = Vec::with_capacity(uids.len());
    for uid_v in uids {
        let uid = match uid_v.as_str() {
            Some(s) => s.to_owned(),
            None => continue,
        };
        let record = match result.get(&uid) {
            Some(r) => r,
            None => continue,
        };

        let accession = record.get("accession").and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let entry_type = record.get("entrytype").and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let n_samples = record.get("n_samples").and_then(serde_json::Value::as_u64).map(|n| n as u32);

        let samples: Vec<GdsSample> = record
            .get("samples")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| serde_json::from_value::<GdsSample>(s.clone()).ok())
                    .filter(|s| !s.accession.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let extrelations: Vec<GdsExtRelation> = record
            .get("extrelations")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| serde_json::from_value::<GdsExtRelation>(r.clone()).ok())
                    .filter(|r| !r.target_object.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        out.push(GdsRecord {
            uid,
            accession,
            entry_type,
            n_samples,
            samples,
            extrelations,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_GSE: &str = r#"{
"header":{"type":"esummary","version":"0.3"},
"result":{"uids":["200056924"],
"200056924":{"uid":"200056924","accession":"GSE56924","entrytype":"GSE","n_samples":96,
"samples":[{"accession":"GSM1371490","title":"sample 1"},{"accession":"GSM1371491","title":"sample 2"}],
"extrelations":[{"relationtype":"SRA","targetobject":"SRP041298","targetftplink":"ftp://..."}]}
}}"#;

    const SAMPLE_GSM: &str = r#"{
"header":{"type":"esummary"},
"result":{"uids":["301371490"],
"301371490":{"uid":"301371490","accession":"GSM1371490","entrytype":"GSM","n_samples":0,
"extrelations":[{"relationtype":"SRA","targetobject":"SRP041298"}]}
}}"#;

    #[test]
    fn parses_gse_record() {
        let recs = parse(SAMPLE_GSE).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.accession, "GSE56924");
        assert_eq!(r.entry_type, "GSE");
        assert_eq!(r.n_samples, Some(96));
        assert_eq!(r.samples.len(), 2);
        assert_eq!(r.samples[0].accession, "GSM1371490");
        assert_eq!(r.extrelations.len(), 1);
        assert_eq!(r.extrelations[0].relation_type, "SRA");
        assert_eq!(r.extrelations[0].target_object, "SRP041298");
    }

    #[test]
    fn parses_gsm_record() {
        let recs = parse(SAMPLE_GSM).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.accession, "GSM1371490");
        assert_eq!(r.entry_type, "GSM");
        assert_eq!(r.extrelations[0].target_object, "SRP041298");
    }

    #[test]
    fn parses_real_gse56924_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/gds_esummary_GSE56924.json"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-gds-esummary GSE56924` first");
        let recs = parse(&body).unwrap();
        assert!(!recs.is_empty());
        let r = recs.iter().find(|r| r.accession == "GSE56924").expect("GSE56924 record");
        assert_eq!(r.entry_type, "GSE");
        assert!(r.n_samples.unwrap_or(0) > 0);
        assert!(!r.samples.is_empty());
        assert!(r.extrelations.iter().any(|e| e.target_object.starts_with("SRP")));
    }

    #[test]
    fn parses_real_gsm1371490_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/gds_esummary_GSM1371490.json"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-gds-esummary GSM1371490` first");
        let recs = parse(&body).unwrap();
        let r = recs.iter().find(|r| r.accession == "GSM1371490").expect("GSM1371490 record");
        assert_eq!(r.entry_type, "GSM");
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib parse::gds_esummary 2>&1 | tail -10`
Expected: 4 tests PASS.

If `parses_real_gsm1371490_fixture` fails because GSM1371490's record doesn't have `extrelations`, the test still expects only `entry_type == "GSM"` and a matching accession — that's fine. The conversion logic in Task 7 will need to handle the case of missing extrelations gracefully.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/parse/mod.rs crates/sradb-core/src/parse/gds_esummary.rs
git commit -m "feat(parse): db=gds esummary JSON parser"
```

## Context for Task 2

Task 1 (commit will be ~`<unknown>` after capture) saved `tests/data/ncbi/gds_*.json`. This task parses them.

The eUtils JSON shape uses string-typed UIDs as keys — that's why we do a two-step lookup (`result.uids` array of strings, then `result.<uid>`). Using `serde_json::Value` rather than typed deserializers is intentional because the per-UID fields vary by entrytype (GSE has `samples`, GSM doesn't).

---

## Task 3: NCBI db=gds wrapper

**Files:**
- Create: `crates/sradb-core/src/ncbi/gds.rs`
- Modify: `crates/sradb-core/src/ncbi/mod.rs`

- [ ] **Step 1: Update ncbi/mod.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/ncbi/mod.rs`. Add `pub mod gds;` (between `efetch` and `esearch`):

```rust
//! Wrappers for NCBI eUtils endpoints.

pub mod efetch;
pub mod esearch;
pub mod esummary;
pub mod gds;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EsearchResult {
    pub count: u64,
    pub webenv: String,
    pub query_key: String,
    pub ids: Vec<String>,
}
```

- [ ] **Step 2: Implement gds.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/ncbi/gds.rs`:

```rust
//! NCBI db=gds (GEO Datasets) wrappers.
//!
//! Two functions, both reusing the existing `esearch` and string-based esummary
//! endpoints with `db=gds`.

use serde::Deserialize;

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};

const CONTEXT_ESEARCH: &str = "gds_esearch";
const CONTEXT_ESUMMARY: &str = "gds_esummary";

#[derive(Debug, Deserialize)]
struct EsearchEnvelope {
    esearchresult: EsearchInner,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct EsearchInner {
    count: String,
    #[serde(default, rename = "idlist")]
    ids: Vec<String>,
}

/// Look up the GDS UID list for one accession (GSE/GSM/GPL).
pub async fn gds_esearch_uids(
    http: &HttpClient,
    base_url: &str,
    accession: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>> {
    let url = format!("{base_url}/esearch.fcgi");
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "gds"),
        ("term", accession),
        ("retmode", "json"),
        ("retmax", "20"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    let env: EsearchEnvelope = http.get_json(CONTEXT_ESEARCH, Service::Ncbi, &url, &q).await?;
    Ok(env.esearchresult.ids)
}

/// Fetch the db=gds esummary JSON for one or more UIDs.
pub async fn gds_esummary_by_uids(
    http: &HttpClient,
    base_url: &str,
    uids: &[String],
    api_key: Option<&str>,
) -> Result<String> {
    if uids.is_empty() {
        return Err(SradbError::Parse {
            endpoint: CONTEXT_ESUMMARY,
            message: "empty UID list".into(),
        });
    }
    let id_param = uids.join(",");
    let url = format!("{base_url}/esummary.fcgi");
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "gds"),
        ("id", &id_param),
        ("retmode", "json"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT_ESUMMARY, Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn esearch_returns_uid_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esearch.fcgi"))
            .and(query_param("db", "gds"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"esearchresult":{"count":"1","idlist":["200056924"]}}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let uids = gds_esearch_uids(&http, &server.uri(), "GSE56924", None).await.unwrap();
        assert_eq!(uids, vec!["200056924".to_string()]);
    }

    #[tokio::test]
    async fn esummary_calls_with_id_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esummary.fcgi"))
            .and(query_param("db", "gds"))
            .and(query_param("id", "200056924"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"result\":{\"uids\":[]}}"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = gds_esummary_by_uids(&http, &server.uri(), &["200056924".to_string()], None).await.unwrap();
        assert!(body.contains("\"uids\""));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib ncbi::gds 2>&1 | tail -10`
Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/ncbi/mod.rs crates/sradb-core/src/ncbi/gds.rs
git commit -m "feat(ncbi): db=gds esearch + esummary wrappers"
```

## Context for Task 3

Reuses the same patterns as `ncbi::esearch` and `ncbi::esummary` from slices 2-3, but pinned to `db=gds`. Returning the raw esummary body (string) keeps the wrapper simple — the `parse::gds_esummary::parse` function (Task 2) handles deserialization.

---

## Task 4: Convert engine — types, lookup table, identity strategy

**Files:**
- Create: `crates/sradb-core/src/convert.rs`
- Modify: `crates/sradb-core/src/lib.rs` (add `pub mod convert;`)

- [ ] **Step 1: Update lib.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/lib.rs`. Add `pub mod convert;` (alphabetical, between `client` and `ena`):

```rust
//! sradb-core — core types, HTTP client, and parsers for the sradb-rs project.
//!
//! See `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` for the full spec.

pub mod accession;
pub mod client;
pub mod convert;
pub mod ena;
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

- [ ] **Step 2: Implement convert.rs (types + lookup, no execution yet)**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/convert.rs`:

```rust
//! Accession conversion engine.
//!
//! Replaces pysradb's 25+ separate conversion methods with a single
//! `(from_kind, to_kind) → Strategy` lookup. Two strategies handle every case:
//! - `ProjectFromMetadata`: call the metadata orchestrator, project a field per row.
//! - `GdsLookup`: call db=gds esearch+esummary, project a field from the JSON.
//!
//! Slice 4 implements both strategies. Chained conversions (e.g. GSE→SRX via
//! GSE→SRP→SRX) are out of scope; those return `UnsupportedConversion`.

use crate::accession::AccessionKind;

/// One field projector for the `ProjectFromMetadata` strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjField {
    StudyAccession,
    ExperimentAccession,
    RunAccession,
    SampleAccession,
    /// GSM accession parsed out of the experiment title (`"GSM3526037: ..."`).
    GeoExperimentFromTitle,
}

/// One field projector for the `GdsLookup` strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GdsField {
    /// `record.accession` for a record where `entrytype == "GSE"`.
    GseAccession,
    /// First `extrelations` entry whose `target_object` starts with `SRP`.
    SrpFromExtrelations,
    /// All child `samples[].accession` values.
    GsmsFromSamples,
    /// For a GSM record: the parent GSE — derived from `extrelations` or by
    /// chaining GSM→SRP→GSE. Slice 4 prefers the chain; documenting the field
    /// for completeness.
    GseFromGsmExtrelations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Diagonal: return the input unchanged.
    Identity,
    ProjectFromMetadata(ProjField),
    GdsLookup(GdsField),
    /// Chain: convert through an intermediate kind first.
    Chain { via: AccessionKind, second: ChainStep },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStep {
    /// After the chain's first leg yields one or more accessions, run convert
    /// again from `via` to the final target.
    Next,
}

/// Look up the strategy for converting `from` → `to`. Returns `None` for
/// unsupported pairs (e.g. GSM→GSM is a no-op via Identity, but GSM→PMID is
/// not supported in slice 4).
#[must_use]
pub fn strategy_for(from: AccessionKind, to: AccessionKind) -> Option<Strategy> {
    use AccessionKind::*;
    if from == to {
        return Some(Strategy::Identity);
    }
    let s = match (from, to) {
        // SRA family ↔ SRA family + GSM via metadata projection
        (Srp, Srx) | (Srr, Srx) | (Srs, Srx) | (Gsm, Srx) => Strategy::ProjectFromMetadata(ProjField::ExperimentAccession),
        (Srp, Srr) | (Srx, Srr) | (Gsm, Srr) => Strategy::ProjectFromMetadata(ProjField::RunAccession),
        (Srp, Srs) | (Srx, Srs) | (Srr, Srs) | (Gsm, Srs) => Strategy::ProjectFromMetadata(ProjField::SampleAccession),
        (Srx, Srp) | (Srr, Srp) | (Gsm, Srp) => Strategy::ProjectFromMetadata(ProjField::StudyAccession),
        (Srx, Gsm) | (Srr, Gsm) | (Srs, Gsm) => Strategy::ProjectFromMetadata(ProjField::GeoExperimentFromTitle),

        // GSE-related: db=gds path
        (Srp, Gse) => Strategy::GdsLookup(GdsField::GseAccession),
        (Gsm, Gse) => Strategy::Chain { via: Srp, second: ChainStep::Next }, // GSM→SRP→GSE
        (Gse, Srp) => Strategy::GdsLookup(GdsField::SrpFromExtrelations),
        (Gse, Gsm) => Strategy::GdsLookup(GdsField::GsmsFromSamples),

        // Chained conversions involving GSE on either side
        (Gse, Srx) | (Gse, Srr) | (Gse, Srs) => Strategy::Chain { via: Srp, second: ChainStep::Next }, // GSE→SRP→target
        (Srs, Srp) => Strategy::Chain { via: Srx, second: ChainStep::Next }, // SRS→SRX→SRP (pysradb skips Srs→Srp directly)

        _ => return None,
    };
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AccessionKind::*;

    #[test]
    fn diagonal_is_identity() {
        for k in [Srp, Srx, Srr, Srs, Gse, Gsm] {
            assert_eq!(strategy_for(k, k), Some(Strategy::Identity), "k={:?}", k);
        }
    }

    #[test]
    fn supported_pairs_have_strategies() {
        // Every cell with a check-mark in the conversion table.
        let pairs = [
            (Srp, Srx), (Srp, Srr), (Srp, Srs), (Srp, Gse),
            (Srx, Srp), (Srx, Srr), (Srx, Srs), (Srx, Gsm),
            (Srr, Srp), (Srr, Srx), (Srr, Srs), (Srr, Gsm),
            (Srs, Srx), (Srs, Gsm),
            (Gse, Srp), (Gse, Gsm),
            (Gsm, Srp), (Gsm, Srx), (Gsm, Srr), (Gsm, Srs), (Gsm, Gse),
        ];
        for (from, to) in pairs {
            assert!(strategy_for(from, to).is_some(), "missing strategy for {:?} → {:?}", from, to);
        }
    }

    #[test]
    fn unsupported_pairs_return_none() {
        assert!(strategy_for(Pmid, Srp).is_none());
        assert!(strategy_for(Srp, Pmid).is_none());
        assert!(strategy_for(Doi, Pmc).is_none());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib convert 2>&1 | tail -10`
Expected: 3 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/convert.rs
git commit -m "feat(convert): strategy lookup table for 25+ conversion pairs"
```

## Context for Task 4

The strategy enum encodes pysradb's full 25-pair conversion table in one declarative `match`. Adding a new conversion = one line. This is the architectural payoff of slice 4.

`Chain { via, second }` exists so that `GSE→{SRX,SRR,SRS}` works without writing extra strategies. The execute step (Task 7) walks the chain by calling `convert` recursively.

`Strategy::Identity` for the diagonal is intentional: callers can ask `convert(SRP_x, Srp)` and get `[SRP_x]` back — useful for code that's generic over `to_kind` and shouldn't special-case the trivial pair.

---

## Task 5: ProjectFromMetadata executor

**Files:**
- Modify: `crates/sradb-core/src/convert.rs` (add execute function for ProjectFromMetadata)

- [ ] **Step 1: Extend the top-of-file imports**

Read `crates/sradb-core/src/convert.rs`. The current `use` block at the top has only `use crate::accession::AccessionKind;`. Replace that line with the full import block we'll need for Tasks 5-7:

```rust
use std::collections::HashSet;

use crate::accession::{Accession, AccessionKind};
use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::metadata;
use crate::model::{MetadataOpts, MetadataRow};
use crate::ncbi::gds as ncbi_gds;
use crate::parse;
```

`SradbError` is unused in the existing Task 4 code but Task 7 will use it. To avoid a warning, add `#[allow(unused_imports)]` above the use block — we remove the allow at the end of Task 7 once everything is wired. Or simpler: add a temporary `_ = SradbError::Placeholder` somewhere... no, just live with the warning until Task 6 (only one warning, doesn't block builds).

Actually, leave the use-block as above and accept the temporary warnings. Run `cargo build` and verify the only warnings are `unused_imports` for the new imports.

- [ ] **Step 2: Append executor function**

Append (do not replace) to `crates/sradb-core/src/convert.rs` after the test module's closing `}`:

```rust

const GSM_TITLE_RE: &str = r"GSM\d{3,}";

fn project_metadata_row(row: &MetadataRow, field: ProjField) -> Option<String> {
    match field {
        ProjField::StudyAccession => non_empty(row.study.accession.clone()),
        ProjField::ExperimentAccession => non_empty(row.experiment.accession.clone()),
        ProjField::RunAccession => non_empty(row.run.accession.clone()),
        ProjField::SampleAccession => non_empty(row.sample.accession.clone()),
        ProjField::GeoExperimentFromTitle => row
            .experiment
            .title
            .as_deref()
            .and_then(extract_gsm),
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn extract_gsm(title: &str) -> Option<String> {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(GSM_TITLE_RE).unwrap());
    RE.find(title).map(|m| m.as_str().to_owned())
}

/// Execute `ProjectFromMetadata`: call `metadata::fetch_metadata` and project the field.
pub async fn execute_project_from_metadata(
    http: &HttpClient,
    ncbi_base_url: &str,
    ena_base_url: &str,
    api_key: Option<&str>,
    input: &Accession,
    field: ProjField,
) -> Result<Vec<String>> {
    let opts = MetadataOpts { detailed: false, enrich: false, page_size: 500 };
    let rows = metadata::fetch_metadata(http, ncbi_base_url, ena_base_url, api_key, &input.raw, &opts).await?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for row in &rows {
        if let Some(v) = project_metadata_row(row, field) {
            if seen.insert(v.clone()) {
                out.push(v);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod project_tests {
    use super::*;

    fn fixture_row() -> MetadataRow {
        use crate::model::{Experiment, Library, Platform, Run, RunUrls, Sample, Study};
        MetadataRow {
            run: Run {
                accession: "SRR8361601".into(),
                experiment_accession: "SRX5172107".into(),
                sample_accession: "SRS4179725".into(),
                study_accession: "SRP174132".into(),
                ..Run::default()
            },
            experiment: Experiment {
                accession: "SRX5172107".into(),
                title: Some("GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq".into()),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                library: Library::default(),
                platform: Platform::default(),
                ..Experiment::default()
            },
            sample: Sample { accession: "SRS4179725".into(), ..Sample::default() },
            study: Study { accession: "SRP174132".into(), ..Study::default() },
            enrichment: None,
        }
    }

    #[test]
    fn project_each_field() {
        let row = fixture_row();
        assert_eq!(project_metadata_row(&row, ProjField::StudyAccession).as_deref(), Some("SRP174132"));
        assert_eq!(project_metadata_row(&row, ProjField::ExperimentAccession).as_deref(), Some("SRX5172107"));
        assert_eq!(project_metadata_row(&row, ProjField::RunAccession).as_deref(), Some("SRR8361601"));
        assert_eq!(project_metadata_row(&row, ProjField::SampleAccession).as_deref(), Some("SRS4179725"));
        assert_eq!(project_metadata_row(&row, ProjField::GeoExperimentFromTitle).as_deref(), Some("GSM3526037"));
    }

    #[test]
    fn extract_gsm_misc() {
        assert_eq!(extract_gsm("GSM12345: bla"), Some("GSM12345".to_string()));
        assert_eq!(extract_gsm("RNA-Seq sample"), None);
        assert_eq!(extract_gsm("preamble GSM999 trailing"), Some("GSM999".to_string()));
    }
}
```

The new imports (`std::collections::HashSet`, `Accession`, `MetadataOpts`, `MetadataRow`, etc.) might collide with imports in the upper part of the file. If so, consolidate at the top of the file (do NOT introduce duplicate imports).

- [ ] **Step 3: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS, with warnings about unused imports (`SradbError`, possibly others — Tasks 6-7 use them).

Run: `cargo test -p sradb-core --lib convert 2>&1 | tail -5`
Expected: 5 tests PASS (3 strategy tests + 2 projection tests).

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/convert.rs
git commit -m "feat(convert): ProjectFromMetadata executor + per-field projection"
```

## Context for Task 5

The `extract_gsm` regex matches `GSM` followed by 3+ digits. This is the same pattern pysradb uses to recover GSM from experiment titles. The 3-digit minimum avoids matching things like `GSM` standalone.

`HashSet` deduplication preserves insertion order via the parallel `Vec<String>` — necessary because pysradb's outputs are deduped but stable.

---

## Task 6: GdsLookup executor

**Files:**
- Modify: `crates/sradb-core/src/convert.rs`

- [ ] **Step 1: Append GdsLookup executor**

Append to the bottom of `crates/sradb-core/src/convert.rs`:

```rust

/// Execute `GdsLookup`: db=gds esearch + esummary, project a field.
pub async fn execute_gds_lookup(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    input: &Accession,
    field: GdsField,
) -> Result<Vec<String>> {
    let uids = ncbi_gds::gds_esearch_uids(http, ncbi_base_url, &input.raw, api_key).await?;
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let body = ncbi_gds::gds_esummary_by_uids(http, ncbi_base_url, &uids, api_key).await?;
    let records = parse::gds_esummary::parse(&body)?;

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for record in &records {
        match field {
            GdsField::GseAccession => {
                if record.entry_type == "GSE" && !record.accession.is_empty() {
                    if seen.insert(record.accession.clone()) {
                        out.push(record.accession.clone());
                    }
                }
            }
            GdsField::SrpFromExtrelations => {
                for rel in &record.extrelations {
                    if rel.target_object.starts_with("SRP") || rel.target_object.starts_with("ERP") || rel.target_object.starts_with("DRP") {
                        if seen.insert(rel.target_object.clone()) {
                            out.push(rel.target_object.clone());
                        }
                    }
                }
            }
            GdsField::GsmsFromSamples => {
                for s in &record.samples {
                    if !s.accession.is_empty() && seen.insert(s.accession.clone()) {
                        out.push(s.accession.clone());
                    }
                }
            }
            GdsField::GseFromGsmExtrelations => {
                // For GSM records, extrelations typically points to SRA, not GSE.
                // Slice 4 prefers the chain GSM→SRP→GSE; this branch is a no-op.
                // Strategy::Chain handles the actual GSM→GSE resolution.
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod gds_executor_tests {
    use super::*;
    use crate::parse::gds_esummary::{GdsExtRelation, GdsRecord, GdsSample};

    fn fake_gse_record() -> GdsRecord {
        GdsRecord {
            uid: "200056924".into(),
            accession: "GSE56924".into(),
            entry_type: "GSE".into(),
            n_samples: Some(2),
            samples: vec![
                GdsSample { accession: "GSM1".into(), title: "s1".into() },
                GdsSample { accession: "GSM2".into(), title: "s2".into() },
            ],
            extrelations: vec![GdsExtRelation { relation_type: "SRA".into(), target_object: "SRP041298".into() }],
        }
    }

    fn project_field(record: &GdsRecord, field: GdsField) -> Vec<String> {
        // Mirror of execute_gds_lookup's per-record projection, factored out for unit testing.
        let mut out = Vec::new();
        match field {
            GdsField::GseAccession => {
                if record.entry_type == "GSE" && !record.accession.is_empty() {
                    out.push(record.accession.clone());
                }
            }
            GdsField::SrpFromExtrelations => {
                for rel in &record.extrelations {
                    if rel.target_object.starts_with("SRP") {
                        out.push(rel.target_object.clone());
                    }
                }
            }
            GdsField::GsmsFromSamples => {
                for s in &record.samples {
                    out.push(s.accession.clone());
                }
            }
            GdsField::GseFromGsmExtrelations => {}
        }
        out
    }

    #[test]
    fn project_gse_accession() {
        let r = fake_gse_record();
        assert_eq!(project_field(&r, GdsField::GseAccession), vec!["GSE56924".to_string()]);
    }

    #[test]
    fn project_srp_from_extrelations() {
        let r = fake_gse_record();
        assert_eq!(project_field(&r, GdsField::SrpFromExtrelations), vec!["SRP041298".to_string()]);
    }

    #[test]
    fn project_gsms_from_samples() {
        let r = fake_gse_record();
        assert_eq!(project_field(&r, GdsField::GsmsFromSamples), vec!["GSM1".to_string(), "GSM2".to_string()]);
    }
}
```

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test -p sradb-core --lib convert 2>&1 | tail -5`
Expected: 8 tests PASS (3 strategy + 2 projection + 3 gds projection).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/convert.rs
git commit -m "feat(convert): GdsLookup executor for GSE↔SRP and GSE↔GSM"
```

## Context for Task 6

The `project_field` test helper duplicates the per-record projection logic from `execute_gds_lookup` so we can unit-test it without HTTP. The integration tests (Task 9) cover the full executor path end-to-end.

`GdsField::GseFromGsmExtrelations` is a no-op — the Chain strategy handles GSM→GSE. We document the field name for future use.

---

## Task 7: Top-level dispatch + chain handling

**Files:**
- Modify: `crates/sradb-core/src/convert.rs`

- [ ] **Step 1: Append dispatch function**

Append to `crates/sradb-core/src/convert.rs` (no new imports needed — the top-of-file block from Task 5 already includes everything):

```rust

/// Top-level dispatch: convert one accession to a list of accessions of the target kind.
///
/// Dedupes the result. Returns an empty vec if the input maps to nothing.
/// Returns `Err(SradbError::UnsupportedConversion { ... })` for un-tabled pairs.
pub async fn convert_one(
    http: &HttpClient,
    ncbi_base_url: &str,
    ena_base_url: &str,
    api_key: Option<&str>,
    input: &Accession,
    to: AccessionKind,
) -> Result<Vec<Accession>> {
    let strategy = strategy_for(input.kind, to).ok_or(SradbError::UnsupportedConversion {
        from: input.kind,
        to,
    })?;
    convert_with_strategy(http, ncbi_base_url, ena_base_url, api_key, input, to, strategy).await
}

#[allow(clippy::too_many_arguments)]
fn convert_with_strategy<'a>(
    http: &'a HttpClient,
    ncbi_base_url: &'a str,
    ena_base_url: &'a str,
    api_key: Option<&'a str>,
    input: &'a Accession,
    to: AccessionKind,
    strategy: Strategy,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Accession>>> + Send + 'a>> {
    Box::pin(async move {
        match strategy {
            Strategy::Identity => Ok(vec![input.clone()]),
            Strategy::ProjectFromMetadata(field) => {
                let raws = execute_project_from_metadata(http, ncbi_base_url, ena_base_url, api_key, input, field).await?;
                Ok(raws.into_iter().map(|raw| Accession { kind: to, raw }).collect())
            }
            Strategy::GdsLookup(field) => {
                let raws = execute_gds_lookup(http, ncbi_base_url, api_key, input, field).await?;
                Ok(raws.into_iter().map(|raw| Accession { kind: to, raw }).collect())
            }
            Strategy::Chain { via, second: ChainStep::Next } => {
                // First leg: input → via
                let first_strategy = strategy_for(input.kind, via).ok_or(SradbError::UnsupportedConversion {
                    from: input.kind,
                    to: via,
                })?;
                let mid = convert_with_strategy(http, ncbi_base_url, ena_base_url, api_key, input, via, first_strategy).await?;
                // Second leg: each via → to
                let second_strategy = strategy_for(via, to).ok_or(SradbError::UnsupportedConversion {
                    from: via,
                    to,
                })?;
                let mut seen = HashSet::new();
                let mut out: Vec<Accession> = Vec::new();
                for mid_acc in &mid {
                    let leg = convert_with_strategy(http, ncbi_base_url, ena_base_url, api_key, mid_acc, to, second_strategy).await?;
                    for a in leg {
                        if seen.insert(a.raw.clone()) {
                            out.push(a);
                        }
                    }
                }
                Ok(out)
            }
        }
    })
}
```

The `Box::pin` + explicit lifetimes are needed because `Strategy::Chain` recursively calls `convert_with_strategy` — recursive `async fn` requires boxing in stable Rust.

- [ ] **Step 2: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/convert.rs
git commit -m "feat(convert): top-level dispatch with chained strategy execution"
```

## Context for Task 7

The recursion handles chains of length 2 (e.g. `GSE→Srp` then `Srp→Srx`). Deeper chains aren't currently in the table — if we ever add `GSE→GSM→SRP` 3-leg chains, this same code would handle them transparently.

The deduplication at the chain join (`seen` set) matters because a single GSE might list 96 GSM samples whose underlying SRA records all share one SRP — without dedup, you'd get 96 copies of the same SRP.

---

## Task 8: SraClient::convert + convert_detailed

**Files:**
- Modify: `crates/sradb-core/src/client.rs`

- [ ] **Step 1: Read current client.rs**

Locate the existing `metadata` and `metadata_many` methods on `impl SraClient`. We add two new methods after them, before the closing `}` of the impl block.

- [ ] **Step 2: Append convert + convert_detailed methods**

Inside `impl SraClient`, after `metadata_many`:

```rust

    /// Convert an accession to one or more accessions of `to_kind`.
    /// Returns an empty vec if the input maps to nothing; returns `Err` for unsupported pairs.
    pub async fn convert(
        &self,
        input: &crate::accession::Accession,
        to_kind: crate::accession::AccessionKind,
    ) -> Result<Vec<crate::accession::Accession>> {
        crate::convert::convert_one(
            &self.http,
            &self.cfg.ncbi_base_url,
            &self.cfg.ena_base_url,
            self.cfg.api_key.as_deref(),
            input,
            to_kind,
        )
        .await
    }

    /// Like `convert` but follows up with a metadata fetch for each result.
    /// Useful when the caller wants both the converted accessions and full
    /// metadata in a single call.
    pub async fn convert_detailed(
        &self,
        input: &crate::accession::Accession,
        to_kind: crate::accession::AccessionKind,
    ) -> Result<Vec<crate::model::MetadataRow>> {
        let converted = self.convert(input, to_kind).await?;
        let opts = crate::model::MetadataOpts { detailed: false, enrich: false, page_size: 500 };
        let mut rows: Vec<crate::model::MetadataRow> = Vec::new();
        for acc in &converted {
            let part = self.metadata(&acc.raw, &opts).await?;
            rows.extend(part);
        }
        Ok(rows)
    }
```

- [ ] **Step 3: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/client.rs
git commit -m "feat(client): SraClient::convert + convert_detailed methods"
```

## Context for Task 8

`convert_detailed` is intentionally simple — it's just `convert` + per-result `metadata`. Could parallelize with `join_all`, but the metadata orchestrator already has internal rate-limiting via the HTTP client's governor, so sequential is safe and easier to reason about.

The `to_kind` parameter takes `AccessionKind` directly (not a string) so callers get clap-validated values from the CLI layer.

---

## Task 9: Wiremock e2e for convert engine

**Files:**
- Create: `crates/sradb-core/tests/convert_e2e.rs`

- [ ] **Step 1: Write the test**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/tests/convert_e2e.rs`:

```rust
//! End-to-end test of the convert engine against captured fixtures.

use sradb_core::accession::{Accession, AccessionKind};
use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn srp_to_srx_via_metadata_projection() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("db", "sra"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .and(query_param("db", "sra"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esummary_body))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();

    let input: Accession = "SRP174132".parse().unwrap();
    let result = client.convert(&input, AccessionKind::Srx).await.unwrap();

    // SRP174132 has 10 experiments → 10 unique SRX accessions.
    assert_eq!(result.len(), 10, "expected 10 SRX accessions, got {}: {:?}", result.len(), result);
    for acc in &result {
        assert_eq!(acc.kind, AccessionKind::Srx);
        assert!(acc.raw.starts_with("SRX"), "{}", acc.raw);
    }
}

#[tokio::test]
async fn gse_to_srp_via_gds_lookup() {
    let workspace = sradb_fixtures::workspace_root();
    let gds_esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/gds_esearch_GSE56924.json")).unwrap();
    let gds_esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/gds_esummary_GSE56924.json")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("db", "gds"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gds_esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .and(query_param("db", "gds"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gds_esummary_body))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();

    let input: Accession = "GSE56924".parse().unwrap();
    let result = client.convert(&input, AccessionKind::Srp).await.unwrap();

    assert!(!result.is_empty(), "expected at least one SRP from GSE56924");
    for acc in &result {
        assert_eq!(acc.kind, AccessionKind::Srp);
        assert!(acc.raw.starts_with("SRP"), "{}", acc.raw);
    }
}

#[tokio::test]
async fn identity_returns_input() {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg).unwrap();
    let input: Accession = "SRP174132".parse().unwrap();
    let result = client.convert(&input, AccessionKind::Srp).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].raw, "SRP174132");
}

#[tokio::test]
async fn unsupported_conversion_errors() {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg).unwrap();
    let input: Accession = "SRP174132".parse().unwrap();
    let err = client.convert(&input, AccessionKind::Pmid).await.unwrap_err();
    assert!(matches!(err, sradb_core::SradbError::UnsupportedConversion { .. }));
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sradb-core --test convert_e2e 2>&1 | tail -10`
Expected: 4 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/convert_e2e.rs
git commit -m "test(core): wiremock e2e for convert engine (metadata-projection + gds-lookup)"
```

## Context for Task 9

Two integration tests cover the two strategies. The identity and unsupported-pair tests don't need network — they're pure dispatch checks.

The `srp_to_srx_via_metadata_projection` test reuses the slice-2 fixtures (esearch + esummary for SRP174132) since `convert(SRP, Srx)` runs the metadata orchestrator under the hood.

---

## Task 10: CLI convert subcommand

**Files:**
- Create: `crates/sradb-cli/src/cmd/convert.rs`
- Modify: `crates/sradb-cli/src/cmd.rs`
- Modify: `crates/sradb-cli/src/main.rs`

- [ ] **Step 1: Update cmd.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd.rs`. Add `pub mod convert;`:

```rust
//! Subcommand handlers.

pub mod convert;
pub mod metadata;
```

- [ ] **Step 2: Create cmd/convert.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/convert.rs`:

```rust
//! `sradb convert <FROM> <TO> <ACCESSION>...` handler.

use clap::Args;
use sradb_core::accession::{Accession, AccessionKind};
use sradb_core::{ClientConfig, SraClient};

/// CLI-friendly value for AccessionKind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliAccKind {
    Srp,
    Srx,
    Srr,
    Srs,
    Gse,
    Gsm,
}

impl From<CliAccKind> for AccessionKind {
    fn from(c: CliAccKind) -> Self {
        match c {
            CliAccKind::Srp => AccessionKind::Srp,
            CliAccKind::Srx => AccessionKind::Srx,
            CliAccKind::Srr => AccessionKind::Srr,
            CliAccKind::Srs => AccessionKind::Srs,
            CliAccKind::Gse => AccessionKind::Gse,
            CliAccKind::Gsm => AccessionKind::Gsm,
        }
    }
}

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// Source accession kind.
    #[arg(value_enum)]
    pub from: CliAccKind,

    /// Target accession kind.
    #[arg(value_enum)]
    pub to: CliAccKind,

    /// One or more accessions of the source kind.
    #[arg(required = true)]
    pub accessions: Vec<String>,
}

pub async fn run(args: ConvertArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let to_kind: AccessionKind = args.to.into();

    let mut had_error = false;
    for raw in &args.accessions {
        let input: Accession = match raw.parse() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("error parsing {raw}: {e}");
                had_error = true;
                continue;
            }
        };
        if input.kind != args.from.into() {
            eprintln!(
                "error: {raw} parses as {:?}, but --from said {:?}",
                input.kind,
                AccessionKind::from(args.from),
            );
            had_error = true;
            continue;
        }
        match client.convert(&input, to_kind).await {
            Ok(results) => {
                if results.is_empty() {
                    eprintln!("warning: no results for {raw}");
                }
                for r in &results {
                    println!("{}\t{}", input.raw, r.raw);
                }
            }
            Err(e) => {
                eprintln!("error converting {raw}: {e}");
                had_error = true;
            }
        }
    }
    if had_error {
        std::process::exit(1);
    }
    Ok(())
}
```

- [ ] **Step 3: Update main.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/src/main.rs`. Find the `Cmd` enum:

```rust
#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print build information and exit.
    Info,
    /// Fetch metadata for one or more accessions.
    Metadata(cmd::metadata::MetadataArgs),
}
```

Add a `Convert` variant:

```rust
#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print build information and exit.
    Info,
    /// Fetch metadata for one or more accessions.
    Metadata(cmd::metadata::MetadataArgs),
    /// Convert accessions between SRA / GEO kinds (e.g. `srp srx SRP174132`).
    Convert(cmd::convert::ConvertArgs),
}
```

Add a match arm in `main()`:

```rust
        Some(Cmd::Convert(args)) => cmd::convert::run(args).await,
```

(Goes alongside the existing `Some(Cmd::Metadata(args)) => ...`).

- [ ] **Step 4: Build**

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 5: Smoke help**

Run: `cargo run -p sradb-cli --quiet -- convert --help 2>&1 | tail -15`
Expected: clap help including positional `<FROM>`, `<TO>`, `<ACCESSIONS>...` with possible values listed.

- [ ] **Step 6: Live smoke test (network)**

Run: `cargo run -p sradb-cli --quiet -- convert srp srx SRP174132 2>&1 | head -12`
Expected: 10 lines of `SRP174132\tSRX...`.

Run: `cargo run -p sradb-cli --quiet -- convert gse srp GSE56924 2>&1 | head -3`
Expected: `GSE56924\tSRP041298` (one line).

Run: `cargo run -p sradb-cli --quiet -- convert gsm srp GSM1371490 2>&1 | head -3`
Expected: `GSM1371490\tSRP041298`.

- [ ] **Step 7: Commit**

```bash
git add crates/sradb-cli/src/cmd.rs crates/sradb-cli/src/cmd/convert.rs crates/sradb-cli/src/main.rs
git commit -m "feat(cli): sradb convert <from> <to> <accession>... subcommand"
```

## Context for Task 10

The `CliAccKind` enum is a CLI-only subset of `AccessionKind` — clap's `value_enum` derive needs a concrete enum for parsing positional args. We exclude `BioProject`, `Pmid`, `Doi`, `Pmc` because slice 4's strategy table doesn't cover them yet (slice 5+ adds those).

Output format is `<input>\t<output>` for easy `awk`/`cut` consumption. If slice 5 adds JSON output for convert, this stays the default.

The `input.kind != args.from.into()` check guards against `sradb convert srp srx GSE56924` where the user mismatched the from-kind and the actual accession.

---

## Task 11: Final verification

**Files:** none changed; verification only.

- [ ] **Step 1: Build**

Run: `cargo build --workspace --all-targets 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 2: Test**

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: PASS, total ≥ 55 (47 from slice 3 + ~10 new).

- [ ] **Step 3: Clippy**

Run: `RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3`
Expected: PASS. If errors surface, fix mechanically (common pattern from slices 2-3: missing backticks in docs, redundant `to_owned`, missing `_` separators in numeric literals).

- [ ] **Step 4: Fmt**

Run: `cargo fmt --all -- --check 2>&1 | tail -2`
Expected: PASS. If fail: `cargo fmt --all` and commit separately.

- [ ] **Step 5: Mark plan complete**

Edit this plan file and add `✅` to each completed task heading.

```bash
git add docs/superpowers/plans/2026-04-26-sradb-rs-slice-4-convert.md
git commit -m "docs(plan): mark slice-4 tasks complete"
```

- [ ] **Step 6: Tag**

```bash
git tag -a slice-4-convert -m "Slice 4: accession conversion engine — sradb convert <from> <to> <acc>"
```

---

## What this slice does NOT include (intentional deferrals)

- `convert --detailed` flag (would chain into the slice-3 detailed orchestrator). Easy to add as polish.
- PMID / DOI / PMC conversions (those need separate slice 5+ work — different APIs).
- Population of `study_geo_accession` / `experiment_geo_accession` in `--detailed` metadata output (could now be wired via the convert engine; deferred to a polish task).
- JSON / NDJSON output from `sradb convert` (slice 5+ if needed).

## Definition of done for slice 4

1. `cargo build --workspace` clean.
2. `cargo test --workspace` clean — ≥55 tests.
3. `cargo clippy -- -Dwarnings` clean.
4. `cargo fmt --check` clean.
5. `sradb convert srp srx SRP174132` against live NCBI returns 10 SRX accessions.
6. `sradb convert gse srp GSE56924` returns `GSE56924\tSRP041298`.
7. `sradb convert gsm srp GSM1371490` returns `GSM1371490\tSRP041298`.
8. Wiremock e2e covers both ProjectFromMetadata and GdsLookup paths.
9. `git tag slice-4-convert` created.
