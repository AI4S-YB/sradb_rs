# sradb-rs Slice 9: Identifier Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land `sradb id <PMID|DOI|PMC>` — extract `GSE`, `SRP`, `PRJNA`, `PMC` accessions from PubMed/PMC fulltext articles. Three input forms supported: PMID, DOI, PMC ID.

**Architecture:** A new `identifier.rs` module with regex-based identifier extraction from PMC fulltext, plus NCBI elink wrappers (`pmid → pmcid`, `doi → pmid`). Three CLI entry points all funnel through PMC fulltext: PMID → elink to PMCID → efetch PMC XML → extract; DOI → elink to PMID → same path; PMC → efetch PMC XML → extract.

**Tech Stack:** existing `quick-xml` for PMC XML parsing, `regex` for identifier patterns, `reqwest` (via `HttpClient`) for the elink/efetch calls.

**Reference:** Slices 1-8 complete. Original spec section "Identifier extraction" + the pysradb `pmc_to_identifiers`/`pmid_to_identifiers`/`doi_to_identifiers` functions.

---

## Background: NCBI elink + efetch wire shapes

### `elink` (link UIDs across NCBI databases)

```
GET {ncbi}/elink.fcgi?dbfrom=pubmed&db=pmc&id=<pmid>&retmode=json
```

Response (JSON):
```json
{
  "linksets": [{
    "linksetdbs": [{
      "linkname": "pubmed_pmc",
      "links": ["10802650"]
    }]
  }]
}
```

For DOI → PMID we use:
```
GET {ncbi}/esearch.fcgi?db=pubmed&term=<doi>[doi]&retmode=json
```

For PMC → PMCID, the input is already a PMC accession (e.g. `PMC10802650`). Strip the `PMC` prefix to get the bare numeric ID for efetch.

### `efetch` against PMC

```
GET {ncbi}/efetch.fcgi?db=pmc&id=<pmcid_numeric>&rettype=full&retmode=xml
```

Returns the full article XML (large, can be 100KB+). We don't fully parse it — we run regex over the entire body to find:
- `GSE\d+` (GEO Series)
- `GSM\d+` (GEO Sample)
- `SRP\d+` (SRA Study)
- `PRJ[A-Z]{2}\d+` (BioProject)

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/identifier.rs` | `IdentifierSet` struct, regex extraction, three extraction entry points |
| `crates/sradb-core/src/lib.rs` | (modify) `pub mod identifier;` |
| `crates/sradb-core/src/ncbi/elink.rs` | NCBI elink wrapper (pubmed → pmc) |
| `crates/sradb-core/src/ncbi/mod.rs` | (modify) `pub mod elink;` |
| `crates/sradb-core/src/client.rs` | (modify) `SraClient::identifiers_from_pmid/doi/pmc` |
| `crates/sradb-cli/src/cmd/id.rs` | CLI handler |
| `crates/sradb-cli/src/cmd.rs` | (modify) `pub mod id;` |
| `crates/sradb-cli/src/main.rs` | (modify) register `Id` subcommand |
| `crates/sradb-core/tests/id_e2e.rs` | Wiremock e2e |

---

## Task 1: IdentifierSet + regex extractor ✅

**Files:**
- Modify: `crates/sradb-core/src/lib.rs`
- Create: `crates/sradb-core/src/identifier.rs`

- [ ] **Step 1: Update lib.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/lib.rs`. Add `pub mod identifier;` (between `http` and `metadata`):

```rust
pub mod accession;
pub mod client;
pub mod convert;
pub mod download;
pub mod ena;
pub mod enrich;
pub mod error;
pub mod geo;
pub mod http;
pub mod identifier;
pub mod metadata;
pub mod model;
pub mod ncbi;
pub mod parse;
pub mod search;

pub use accession::{Accession, AccessionKind, ParseAccessionError};
pub use client::{ClientConfig, SraClient};
pub use error::{Result, SradbError};
pub use identifier::IdentifierSet;
pub use model::{
    Enrichment, Experiment, Library, LibraryLayout, MetadataOpts, MetadataRow, Platform, Run,
    RunUrls, Sample, Study,
};
```

- [ ] **Step 2: Create identifier.rs (types + regex extractor only)**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/identifier.rs`:

```rust
//! Extract database identifiers (GSE, GSM, SRP, PRJNA) from PMC fulltext.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Identifiers found in a PubMed / PMC article.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IdentifierSet {
    /// Source PubMed ID.
    pub pmid: Option<u64>,
    /// Source PMC ID (with `PMC` prefix).
    pub pmc_id: Option<String>,
    /// Source DOI.
    pub doi: Option<String>,
    pub gse_ids: Vec<String>,
    pub gsm_ids: Vec<String>,
    pub srp_ids: Vec<String>,
    pub prjna_ids: Vec<String>,
}

/// Run all four regexes over the body and populate ID lists. Existing fields
/// (pmid, pmc_id, doi) are not modified — set them externally.
pub fn extract_into(body: &str, set: &mut IdentifierSet) {
    static GSE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bGSE\d{2,}\b").unwrap());
    static GSM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bGSM\d{3,}\b").unwrap());
    static SRP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[ESDR]?SRP\d{4,}\b|\b[EDS]RP\d{4,}\b").unwrap());
    static PRJ_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bPRJ[A-Z]{2}\d{4,}\b").unwrap());

    set.gse_ids = dedup_sorted(GSE_RE.find_iter(body).map(|m| m.as_str().to_owned()));
    set.gsm_ids = dedup_sorted(GSM_RE.find_iter(body).map(|m| m.as_str().to_owned()));
    set.srp_ids = dedup_sorted(SRP_RE.find_iter(body).map(|m| m.as_str().to_owned()));
    set.prjna_ids = dedup_sorted(PRJ_RE.find_iter(body).map(|m| m.as_str().to_owned()));
}

fn dedup_sorted(items: impl Iterator<Item = String>) -> Vec<String> {
    let set: BTreeSet<String> = items.collect();
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_gse_and_srp() {
        let body = "Data deposited at GSE253406 and SRP484103 are publicly available.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert_eq!(set.gse_ids, vec!["GSE253406".to_string()]);
        assert_eq!(set.srp_ids, vec!["SRP484103".to_string()]);
    }

    #[test]
    fn extracts_prjna() {
        let body = "BioProject PRJNA1058002 contains the raw reads.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert_eq!(set.prjna_ids, vec!["PRJNA1058002".to_string()]);
    }

    #[test]
    fn dedup_and_sort() {
        let body = "We used GSE100 and GSE99 and GSE100 again, plus GSE99.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert_eq!(set.gse_ids, vec!["GSE100".to_string(), "GSE99".to_string()]);
    }

    #[test]
    fn empty_body_yields_empty_lists() {
        let mut set = IdentifierSet::default();
        extract_into("", &mut set);
        assert!(set.gse_ids.is_empty());
        assert!(set.srp_ids.is_empty());
        assert!(set.prjna_ids.is_empty());
        assert!(set.gsm_ids.is_empty());
    }

    #[test]
    fn skips_partial_matches() {
        let body = "GSE12 is too short, GSE123 is fine. SRP12 too short, SRP1234 fine.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert!(!set.gse_ids.contains(&"GSE12".to_string()));
        assert!(set.gse_ids.contains(&"GSE123".to_string()));
        assert!(set.srp_ids.contains(&"SRP1234".to_string()));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib identifier 2>&1 | tail -10`
Expected: 5 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/identifier.rs
git commit -m "feat(identifier): IdentifierSet + regex extraction (GSE/GSM/SRP/PRJNA)"
```

---

## Task 2: NCBI elink wrapper ✅

**Files:**
- Modify: `crates/sradb-core/src/ncbi/mod.rs`
- Create: `crates/sradb-core/src/ncbi/elink.rs`

- [ ] **Step 1: Update ncbi/mod.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/ncbi/mod.rs`. Currently has `efetch`, `esearch`, `esummary`, `gds`. Add `pub mod elink;` (alphabetical first):

```rust
//! Wrappers for NCBI eUtils endpoints.

pub mod efetch;
pub mod elink;
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

- [ ] **Step 2: Create elink.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/ncbi/elink.rs`:

```rust
//! NCBI elink wrapper: link UIDs across databases (e.g. pubmed → pmc).

use serde::Deserialize;

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT: &str = "elink";

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Envelope {
    linksets: Vec<LinkSet>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct LinkSet {
    linksetdbs: Vec<LinkSetDb>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct LinkSetDb {
    linkname: String,
    links: Vec<String>,
}

/// Link a PubMed ID to PMC IDs. Returns the PMC numeric IDs (without "PMC" prefix).
pub async fn pmid_to_pmc_ids(
    http: &HttpClient,
    base_url: &str,
    pmid: u64,
    api_key: Option<&str>,
) -> Result<Vec<String>> {
    let url = format!("{base_url}/elink.fcgi");
    let pmid_s = pmid.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("dbfrom", "pubmed"),
        ("db", "pmc"),
        ("id", &pmid_s),
        ("retmode", "json"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    let env: Envelope = http.get_json(CONTEXT, Service::Ncbi, &url, &q).await?;
    let mut out: Vec<String> = Vec::new();
    for ls in &env.linksets {
        for db in &ls.linksetdbs {
            if db.linkname == "pubmed_pmc" {
                out.extend(db.links.iter().cloned());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn pmid_to_pmc_ids_extracts_link() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/elink.fcgi"))
            .and(query_param("dbfrom", "pubmed"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"linksets":[{"dbfrom":"pubmed","ids":["39528918"],"linksetdbs":[{"dbto":"pmc","linkname":"pubmed_pmc","links":["10802650"]}]}]}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let pmcs = pmid_to_pmc_ids(&http, &server.uri(), 39_528_918, None).await.unwrap();
        assert_eq!(pmcs, vec!["10802650".to_string()]);
    }

    #[tokio::test]
    async fn pmid_with_no_pmc_link_returns_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/elink.fcgi"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"linksets":[{"dbfrom":"pubmed","ids":["1"],"linksetdbs":[]}]}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let pmcs = pmid_to_pmc_ids(&http, &server.uri(), 1, None).await.unwrap();
        assert!(pmcs.is_empty());
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p sradb-core --lib ncbi::elink 2>&1 | tail -5`
Expected: 2 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/ncbi/mod.rs crates/sradb-core/src/ncbi/elink.rs
git commit -m "feat(ncbi): elink wrapper for pubmed → pmc"
```

---

## Task 3: Identifier extraction entry points ✅

**Files:**
- Modify: `crates/sradb-core/src/identifier.rs`

- [ ] **Step 1: Append three entry points**

Append to `/home/xzg/project/sradb_rs/crates/sradb-core/src/identifier.rs`:

```rust

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};
use crate::ncbi::{elink, esearch};

/// PubMed → PMC → fulltext → identifiers.
pub async fn from_pmid(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    pmid: u64,
) -> Result<IdentifierSet> {
    let mut set = IdentifierSet { pmid: Some(pmid), ..IdentifierSet::default() };
    let pmc_ids = elink::pmid_to_pmc_ids(http, base_url, pmid, api_key).await?;
    if let Some(pmc_numeric) = pmc_ids.first() {
        set.pmc_id = Some(format!("PMC{pmc_numeric}"));
        let body = fetch_pmc_fulltext(http, base_url, pmc_numeric, api_key).await?;
        extract_into(&body, &mut set);
    }
    Ok(set)
}

/// DOI → PMID → PMC → identifiers.
pub async fn from_doi(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    doi: &str,
) -> Result<IdentifierSet> {
    // esearch on db=pubmed with `<doi>[doi]` term.
    let term = format!("{doi}[doi]");
    let result = esearch::esearch(http, base_url, "pubmed", &term, api_key, 1).await?;
    let pmid_str = result.ids.first().ok_or_else(|| SradbError::NotFound(format!("DOI {doi} not in PubMed")))?;
    let pmid: u64 = pmid_str.parse().map_err(|_| SradbError::Parse {
        endpoint: "doi_to_pmid",
        message: format!("non-numeric PMID `{pmid_str}`"),
    })?;
    let mut set = from_pmid(http, base_url, api_key, pmid).await?;
    set.doi = Some(doi.to_owned());
    Ok(set)
}

/// PMC → fulltext → identifiers.
pub async fn from_pmc(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    pmc: &str,
) -> Result<IdentifierSet> {
    let pmc_numeric = pmc.trim_start_matches("PMC");
    if pmc_numeric.is_empty() || !pmc_numeric.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SradbError::InvalidAccession {
            input: pmc.to_owned(),
            reason: "expected PMC<digits>".into(),
        });
    }
    let mut set = IdentifierSet { pmc_id: Some(format!("PMC{pmc_numeric}")), ..IdentifierSet::default() };
    let body = fetch_pmc_fulltext(http, base_url, pmc_numeric, api_key).await?;
    extract_into(&body, &mut set);
    Ok(set)
}

async fn fetch_pmc_fulltext(
    http: &HttpClient,
    base_url: &str,
    pmc_numeric: &str,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/efetch.fcgi");
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "pmc"),
        ("id", pmc_numeric),
        ("rettype", "full"),
        ("retmode", "xml"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text("efetch_pmc", Service::Ncbi, &url, &q).await
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/identifier.rs
git commit -m "feat(identifier): from_pmid / from_doi / from_pmc entry points"
```

---

## Task 4: SraClient methods + CLI ✅

**Files:**
- Modify: `crates/sradb-core/src/client.rs`
- Modify: `crates/sradb-cli/src/cmd.rs`
- Create: `crates/sradb-cli/src/cmd/id.rs`
- Modify: `crates/sradb-cli/src/main.rs`

- [ ] **Step 1: Append SraClient methods**

Inside `impl SraClient`, after `enrich_rows` (or after `geo_matrix_download` if slice 8 not yet merged):

```rust

    /// Extract database identifiers from a PubMed PMID.
    pub async fn identifiers_from_pmid(&self, pmid: u64) -> Result<crate::identifier::IdentifierSet> {
        crate::identifier::from_pmid(
            &self.http, &self.cfg.ncbi_base_url, self.cfg.api_key.as_deref(), pmid,
        ).await
    }

    /// Extract database identifiers from a DOI (resolves to PMID then PMC).
    pub async fn identifiers_from_doi(&self, doi: &str) -> Result<crate::identifier::IdentifierSet> {
        crate::identifier::from_doi(
            &self.http, &self.cfg.ncbi_base_url, self.cfg.api_key.as_deref(), doi,
        ).await
    }

    /// Extract database identifiers from a PMC ID.
    pub async fn identifiers_from_pmc(&self, pmc: &str) -> Result<crate::identifier::IdentifierSet> {
        crate::identifier::from_pmc(
            &self.http, &self.cfg.ncbi_base_url, self.cfg.api_key.as_deref(), pmc,
        ).await
    }
```

- [ ] **Step 2: Update cmd.rs**

```rust
//! Subcommand handlers.

pub mod convert;
pub mod download;
pub mod geo;
pub mod id;
pub mod metadata;
pub mod search;
```

- [ ] **Step 3: Create cmd/id.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/id.rs`:

```rust
//! `sradb id <PMID|DOI|PMC>` handler.

use clap::Args;
use sradb_core::{ClientConfig, SraClient};

#[derive(Args, Debug)]
pub struct IdArgs {
    /// One identifier (PMID number, PMC accession like `PMC10802650`, or DOI like `10.1234/abcd`).
    pub identifier: String,

    /// Output as JSON instead of plaintext.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub async fn run(args: IdArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let id = args.identifier.trim();

    let set = if id.starts_with("PMC") {
        client.identifiers_from_pmc(id).await?
    } else if id.starts_with("10.") {
        client.identifiers_from_doi(id).await?
    } else if let Ok(pmid) = id.parse::<u64>() {
        client.identifiers_from_pmid(pmid).await?
    } else {
        return Err(anyhow::anyhow!("unrecognized identifier: {id} (expected PMID number, PMC<digits>, or DOI starting with 10.)"));
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&set)?);
    } else {
        if let Some(p) = set.pmid { println!("pmid:\t{p}"); }
        if let Some(p) = &set.pmc_id { println!("pmc:\t{p}"); }
        if let Some(d) = &set.doi { println!("doi:\t{d}"); }
        for g in &set.gse_ids { println!("gse:\t{g}"); }
        for g in &set.gsm_ids { println!("gsm:\t{g}"); }
        for s in &set.srp_ids { println!("srp:\t{s}"); }
        for p in &set.prjna_ids { println!("prjna:\t{p}"); }
    }
    Ok(())
}
```

- [ ] **Step 4: Update main.rs**

In the `Cmd` enum (after `Geo`):

```rust
    /// Extract database identifiers from PMID / DOI / PMC.
    Id(cmd::id::IdArgs),
```

In the match block:

```rust
        Some(Cmd::Id(args)) => cmd::id::run(args).await,
```

- [ ] **Step 5: Build + smoke help**

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: PASS.

Run: `cargo run -p sradb-cli --quiet -- id --help 2>&1 | tail -10`
Expected: clap help showing `<IDENTIFIER>` and `--json`.

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-core/src/client.rs crates/sradb-cli/src/cmd.rs crates/sradb-cli/src/cmd/id.rs crates/sradb-cli/src/main.rs
git commit -m "feat(cli): sradb id <PMID|DOI|PMC> with auto-detection + --json"
```

---

## Task 5: Wiremock e2e ✅

**Files:**
- Create: `crates/sradb-core/tests/id_e2e.rs`

- [ ] **Step 1: Write the test**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/tests/id_e2e.rs`:

```rust
//! End-to-end test of identifier extraction against wiremock.

use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PMC_BODY: &str = r#"<article>
The processed data are deposited at GEO under accession GSE253406 and
the raw reads at SRA under SRP484103. BioProject PRJNA1058002 covers
both. Sample-level deposits include GSM12345 and GSM12346.
</article>"#;

#[tokio::test]
async fn from_pmc_extracts_identifiers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .and(query_param("db", "pmc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PMC_BODY))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let set = client.identifiers_from_pmc("PMC10802650").await.unwrap();
    assert_eq!(set.pmc_id.as_deref(), Some("PMC10802650"));
    assert_eq!(set.gse_ids, vec!["GSE253406".to_string()]);
    assert_eq!(set.srp_ids, vec!["SRP484103".to_string()]);
    assert_eq!(set.prjna_ids, vec!["PRJNA1058002".to_string()]);
    assert_eq!(set.gsm_ids, vec!["GSM12345".to_string(), "GSM12346".to_string()]);
}

#[tokio::test]
async fn from_pmid_chains_through_elink_and_efetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/elink.fcgi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"linksets":[{"linksetdbs":[{"linkname":"pubmed_pmc","links":["10802650"]}]}]}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PMC_BODY))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let set = client.identifiers_from_pmid(39_528_918).await.unwrap();
    assert_eq!(set.pmid, Some(39_528_918));
    assert_eq!(set.pmc_id.as_deref(), Some("PMC10802650"));
    assert_eq!(set.gse_ids, vec!["GSE253406".to_string()]);
}

#[tokio::test]
async fn from_pmc_invalid_input_errors() {
    let server = MockServer::start().await;
    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    assert!(client.identifiers_from_pmc("not-a-pmc").await.is_err());
    assert!(client.identifiers_from_pmc("PMC").await.is_err());
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sradb-core --test id_e2e 2>&1 | tail -10`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/id_e2e.rs
git commit -m "test(identifier): wiremock e2e for from_pmc + from_pmid + invalid input"
```

---

## Task 6: Final verification ✅

- [ ] **Step 1: All gates**

```bash
cargo build --workspace --all-targets 2>&1 | tail -3
cargo fmt --all -- --check 2>&1 | tail -2
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -3
```

Apply mechanical fixes if needed. Commit fixes.

- [ ] **Step 2: Mark + tag**

Add `✅` to all task headings in this plan.

```bash
git add docs/superpowers/plans/2026-04-26-sradb-rs-slice-9-id-extraction.md
git commit -m "docs(plan): mark all 6 slice-9 tasks complete"
git tag -a slice-9-id-extraction -m "Slice 9: identifier extraction from PMID / DOI / PMC"
```

---

## Definition of done

- `cargo test --workspace` ≥98 tests
- Wiremock e2e covers PMC fulltext extraction + PMID chain + invalid input
- `sradb id PMC10802650 --json` against live NCBI returns at least one of GSE/SRP/PRJNA
