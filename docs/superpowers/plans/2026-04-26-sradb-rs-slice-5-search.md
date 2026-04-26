# sradb-rs Slice 5: Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land `sradb search` against the SRA database — `--query`, `--organism`, `--strategy`, `--platform`, `--source`, `--selection`, `--layout`, `--max` filters, with TSV / JSON output. Reuses the slice-2 metadata orchestrator after building an esearch term from the filters.

**Architecture:** A new `search.rs` module that builds NCBI esearch terms from typed `SearchQuery` filters using SRA Entrez field qualifiers (e.g. `Homo sapiens[ORGN] AND RNA-Seq[STRA]`). Search executes esearch → esummary → metadata rows, just like `metadata` but with a constructed term. Output reuses the existing TSV/JSON writers.

**Tech Stack:** existing slice-2 orchestrator, `clap` for CLI, no new external deps.

**Reference:** Spec at `docs/superpowers/specs/2026-04-25-sradb-rs-design.md`. Slices 1-4 complete. ENA portal search and GEO datasets search are deferred to slice 5b/5c (out of scope here).

---

## Background: SRA Entrez field qualifiers

NCBI's eUtils search supports query refinement via field qualifiers. We use these:

| Field | Qualifier | Notes |
| --- | --- | --- |
| Free text | (none — bare term) | matches all fields |
| Organism | `[ORGN]` | scientific name, e.g. `Homo sapiens[ORGN]` |
| Library strategy | `[STRA]` | e.g. `RNA-Seq[STRA]`, `WGS[STRA]`, `ChIP-Seq[STRA]` |
| Platform | `[PLAT]` | e.g. `ILLUMINA[PLAT]`, `OXFORD_NANOPORE[PLAT]` |
| Library source | `[SRC]` | e.g. `TRANSCRIPTOMIC[SRC]` |
| Library selection | `[SEL]` | e.g. `cDNA[SEL]` |
| Library layout | `[LAY]` | `SINGLE[LAY]` or `PAIRED[LAY]` |

Multiple qualifiers combine with `AND`. Each value is wrapped in quotes if it contains a space.

Example built term:
```
"Homo sapiens"[ORGN] AND "RNA-Seq"[STRA] AND "Illumina HiSeq 2000"[PLAT]
```

The search count limit comes from `--max` (default 20, max 500 per page; pysradb's verbosity 2 default is 20).

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/search.rs` | `SearchQuery` struct + `build_term(query)` + `search(client, query)` |
| `crates/sradb-core/src/lib.rs` | (modify) `pub mod search;` |
| `crates/sradb-core/src/client.rs` | (modify) `SraClient::search` method |
| `crates/sradb-cli/src/cmd/search.rs` | CLI handler (new file) |
| `crates/sradb-cli/src/cmd.rs` | (modify) `pub mod search;` |
| `crates/sradb-cli/src/main.rs` | (modify) register `Search` subcommand |
| `crates/sradb-core/tests/search_e2e.rs` | Wiremock e2e: query-term construction + result parsing |

---

## Task 1: SearchQuery + build_term

**Files:**
- Create: `crates/sradb-core/src/search.rs`
- Modify: `crates/sradb-core/src/lib.rs` (add `pub mod search;`)

- [ ] **Step 1: Update lib.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/lib.rs`. Add `pub mod search;` (alphabetical, between `parse` and the re-exports — actually it goes between `parse` line and the `pub use` block):

```rust
//! sradb-core — core types, HTTP client, and parsers for the sradb-rs project.

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
pub mod search;

pub use accession::{Accession, AccessionKind, ParseAccessionError};
pub use client::{ClientConfig, SraClient};
pub use error::{Result, SradbError};
pub use model::{
    Enrichment, Experiment, Library, LibraryLayout, MetadataOpts, MetadataRow, Platform, Run,
    RunUrls, Sample, Study,
};
```

- [ ] **Step 2: Implement search.rs (term builder + types only)**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/search.rs`:

```rust
//! Search the NCBI SRA database via esearch + esummary.
//!
//! Supports the same filter set as pysradb's `SraSearch`. Builds an Entrez
//! query term with field qualifiers, then runs the metadata orchestrator on
//! the resulting accession set.

use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::metadata;
use crate::model::{MetadataOpts, MetadataRow};
use crate::ncbi::{esearch, esummary};
use crate::parse;

/// Search filters. All fields are optional; an empty query (no filters and no
/// free-text query) returns an error.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Free-text term (no field qualifier).
    pub query: Option<String>,
    /// Organism scientific name, e.g. `"Homo sapiens"`. Maps to `[ORGN]`.
    pub organism: Option<String>,
    /// Library strategy, e.g. `"RNA-Seq"`. Maps to `[STRA]`.
    pub strategy: Option<String>,
    /// Library source, e.g. `"TRANSCRIPTOMIC"`. Maps to `[SRC]`.
    pub source: Option<String>,
    /// Library selection, e.g. `"cDNA"`. Maps to `[SEL]`.
    pub selection: Option<String>,
    /// Library layout, `"SINGLE"` or `"PAIRED"`. Maps to `[LAY]`.
    pub layout: Option<String>,
    /// Platform, e.g. `"ILLUMINA"`. Maps to `[PLAT]`.
    pub platform: Option<String>,
    /// Max results to return (default 20, NCBI hard cap 500 per page).
    pub max: u32,
}

impl SearchQuery {
    #[must_use]
    pub fn new() -> Self {
        Self { max: 20, ..Self::default() }
    }
}

/// Build an Entrez query term from a `SearchQuery`. Returns `None` if the query
/// is empty (no filters, no free text).
#[must_use]
pub fn build_term(q: &SearchQuery) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(t) = &q.query {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(quote_if_needed(t));
        }
    }
    push_qualifier(&mut parts, q.organism.as_deref(), "ORGN");
    push_qualifier(&mut parts, q.strategy.as_deref(), "STRA");
    push_qualifier(&mut parts, q.source.as_deref(), "SRC");
    push_qualifier(&mut parts, q.selection.as_deref(), "SEL");
    push_qualifier(&mut parts, q.layout.as_deref(), "LAY");
    push_qualifier(&mut parts, q.platform.as_deref(), "PLAT");

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" AND "))
    }
}

fn push_qualifier(parts: &mut Vec<String>, value: Option<&str>, qualifier: &str) {
    if let Some(v) = value {
        let v = v.trim();
        if !v.is_empty() {
            parts.push(format!("{}[{}]", quote_if_needed(v), qualifier));
        }
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.contains(char::is_whitespace) || s.contains('-') {
        format!("\"{s}\"")
    } else {
        s.to_owned()
    }
}

/// Run a search end-to-end: esearch (with constructed term) → esummary →
/// metadata rows. Returns up to `query.max` results.
pub async fn search(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    query: &SearchQuery,
) -> Result<Vec<MetadataRow>> {
    let term = build_term(query).ok_or_else(|| SradbError::Parse {
        endpoint: "search",
        message: "empty search query (no filters and no free text)".into(),
    })?;
    let max = if query.max == 0 { 20 } else { query.max.min(500) };

    let result = esearch::esearch(http, ncbi_base_url, "sra", &term, api_key, max).await?;
    if result.count == 0 {
        return Ok(Vec::new());
    }
    if result.webenv.is_empty() || result.query_key.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "search/esearch",
            message: format!("count={} but missing webenv/query_key", result.count),
        });
    }

    let body = esummary::esummary_with_history(
        http, ncbi_base_url, "sra", &result.webenv, &result.query_key,
        0, max, api_key,
    ).await?;
    let docs = parse::esummary::parse(&body)?;

    let mut rows: Vec<MetadataRow> = Vec::new();
    for d in docs {
        rows.extend(metadata::assemble_default_rows(d)?);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_yields_none() {
        assert!(build_term(&SearchQuery::new()).is_none());
    }

    #[test]
    fn single_qualifier() {
        let q = SearchQuery {
            organism: Some("Homo sapiens".into()),
            ..SearchQuery::new()
        };
        assert_eq!(build_term(&q).as_deref(), Some(r#""Homo sapiens"[ORGN]"#));
    }

    #[test]
    fn multiple_qualifiers_joined_by_and() {
        let q = SearchQuery {
            organism: Some("Homo sapiens".into()),
            strategy: Some("RNA-Seq".into()),
            platform: Some("ILLUMINA".into()),
            ..SearchQuery::new()
        };
        let term = build_term(&q).unwrap();
        assert!(term.contains(r#""Homo sapiens"[ORGN]"#));
        assert!(term.contains(r#""RNA-Seq"[STRA]"#));
        assert!(term.contains("ILLUMINA[PLAT]"));
        assert_eq!(term.matches(" AND ").count(), 2);
    }

    #[test]
    fn free_text_only() {
        let q = SearchQuery {
            query: Some("ARID1A breast cancer".into()),
            ..SearchQuery::new()
        };
        assert_eq!(build_term(&q).as_deref(), Some(r#""ARID1A breast cancer""#));
    }

    #[test]
    fn quote_if_needed_skips_unicode_safe_words() {
        assert_eq!(quote_if_needed("ILLUMINA"), "ILLUMINA");
        assert_eq!(quote_if_needed("Homo sapiens"), "\"Homo sapiens\"");
        assert_eq!(quote_if_needed("RNA-Seq"), "\"RNA-Seq\"");
    }

    #[test]
    fn empty_strings_are_skipped() {
        let q = SearchQuery {
            query: Some("   ".into()),
            organism: Some("".into()),
            ..SearchQuery::new()
        };
        assert!(build_term(&q).is_none());
    }
}
```

The function `metadata::assemble_default_rows` doesn't exist yet — Task 2 promotes the existing private `assemble_rows` from `metadata.rs` to a `pub(crate)` API.

- [ ] **Step 3: Build (will fail — Task 2 fixes)**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: error "function `assemble_default_rows` is private" or "cannot find function `assemble_default_rows`". That's expected — Task 2 promotes it.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/search.rs
git commit -m "feat(search): SearchQuery + build_term + search facade (workspace temporarily broken)"
```

## Context for Task 1

We rely on the existing `metadata::assemble_default_rows` which is currently named `assemble_rows` and is private. Task 2 will promote and rename it.

The build is briefly broken between Task 1 and Task 2 — this matches the pattern from slice 3 Task 9.

Working dir: `/home/xzg/project/sradb_rs/`. Branch: `slice-5-search`. HEAD is the slice-4 tag.

---

## Task 2: Promote `assemble_rows` to `pub(crate) assemble_default_rows`

**Files:**
- Modify: `/home/xzg/project/sradb_rs/crates/sradb-core/src/metadata.rs`

- [ ] **Step 1: Read metadata.rs**

Find this private function:

```rust
fn assemble_rows(doc: parse::esummary::RawDocSum) -> Result<Vec<MetadataRow>> {
    ...
}
```

- [ ] **Step 2: Rename and promote**

Change the signature to:

```rust
pub(crate) fn assemble_default_rows(doc: parse::esummary::RawDocSum) -> Result<Vec<MetadataRow>> {
```

(Just rename `assemble_rows` → `assemble_default_rows` and add `pub(crate)`. The body is unchanged.)

- [ ] **Step 3: Update the caller in `fetch_metadata`**

Same file. Find:

```rust
        for d in docs {
            rows.extend(assemble_rows(d)?);
        }
```

Change to:

```rust
        for d in docs {
            rows.extend(assemble_default_rows(d)?);
        }
```

- [ ] **Step 4: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS. The search.rs reference now resolves.

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: existing tests still pass + 6 new search tests (`empty_query_yields_none`, `single_qualifier`, `multiple_qualifiers_joined_by_and`, `free_text_only`, `quote_if_needed_skips_unicode_safe_words`, `empty_strings_are_skipped`).

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-core/src/metadata.rs
git commit -m "refactor(metadata): promote assemble_default_rows to pub(crate) for search reuse"
```

## Context for Task 2

This is a minimal mechanical refactor. The function body doesn't change — only its name and visibility.

---

## Task 3: SraClient::search method

**Files:**
- Modify: `/home/xzg/project/sradb_rs/crates/sradb-core/src/client.rs`

- [ ] **Step 1: Append search method**

Inside the existing `impl SraClient { ... }` block, after `convert_detailed` and before the closing `}`:

```rust

    /// Search SRA via Entrez query terms built from a `SearchQuery`.
    pub async fn search(
        &self,
        query: &crate::search::SearchQuery,
    ) -> Result<Vec<crate::model::MetadataRow>> {
        crate::search::search(
            &self.http,
            &self.cfg.ncbi_base_url,
            self.cfg.api_key.as_deref(),
            query,
        )
        .await
    }
```

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/client.rs
git commit -m "feat(client): SraClient::search method"
```

## Context for Task 3

Thin facade matching the pattern of `metadata`, `metadata_many`, `convert`, `convert_detailed`.

---

## Task 4: CLI search subcommand

**Files:**
- Create: `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/search.rs`
- Modify: `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd.rs`
- Modify: `/home/xzg/project/sradb_rs/crates/sradb-cli/src/main.rs`

- [ ] **Step 1: Update cmd.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd.rs`. Add `pub mod search;`:

```rust
//! Subcommand handlers.

pub mod convert;
pub mod metadata;
pub mod search;
```

- [ ] **Step 2: Create cmd/search.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/search.rs`:

```rust
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
```

- [ ] **Step 3: Update main.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/src/main.rs`. In the `Cmd` enum (after `Convert`):

```rust
    /// Search SRA with field-qualified Entrez queries.
    Search(cmd::search::SearchArgs),
```

In the `match cli.command` block, add (after the Convert arm):

```rust
        Some(Cmd::Search(args)) => cmd::search::run(args).await,
```

- [ ] **Step 4: Build + smoke help**

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

Run: `cargo run -p sradb-cli --quiet -- search --help 2>&1 | tail -20`
Expected: clap help showing `--query`, `--organism`, `--strategy`, `--source`, `--selection`, `--layout`, `--platform`, `--max`, `--format`.

- [ ] **Step 5: Live smoke test**

Run: `cargo run -p sradb-cli --quiet -- search --organism "Homo sapiens" --strategy "RNA-Seq" --max 3 --format json 2>&1 | head -15`
Expected: a JSON array with up to 3 entries, each a `MetadataRow`. Run accessions start with `SRR`, organism is `Homo sapiens`, library strategy is `RNA-Seq`.

If network is blocked, DONE_WITH_CONCERNS — Task 5 covers correctness with wiremock.

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-cli/src/cmd.rs crates/sradb-cli/src/cmd/search.rs crates/sradb-cli/src/main.rs
git commit -m "feat(cli): sradb search with --query / --organism / --strategy / --platform / --layout / --selection / --source / --max / --format"
```

## Context for Task 4

`output::write(rows, format, false, ...)` — the `false` is the `detailed` flag (unused in search results since `assemble_default_rows` only populates default-mode fields).

---

## Task 5: Wiremock e2e for search

**Files:**
- Create: `/home/xzg/project/sradb_rs/crates/sradb-core/tests/search_e2e.rs`

- [ ] **Step 1: Write the test**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/tests/search_e2e.rs`:

```rust
//! End-to-end test of the search engine against captured fixtures.

use sradb_core::search::SearchQuery;
use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn search_executes_and_parses_results() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
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
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let query = SearchQuery {
        organism: Some("Homo sapiens".into()),
        strategy: Some("RNA-Seq".into()),
        ..SearchQuery::new()
    };
    let rows = client.search(&query).await.unwrap();
    assert!(!rows.is_empty(), "expected ≥ 1 row from search");
    for row in &rows {
        assert_eq!(row.sample.organism_name.as_deref(), Some("Homo sapiens"));
        assert_eq!(row.experiment.library.strategy.as_deref(), Some("RNA-Seq"));
    }
}

#[tokio::test]
async fn empty_query_returns_error() {
    let server = MockServer::start().await;
    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let err = client.search(&SearchQuery::new()).await.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("empty search query"), "unexpected: {msg}");
}

#[tokio::test]
async fn esearch_term_includes_orgn_and_stra_qualifiers() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    // Match the term param to assert the constructed query includes ORGN.
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("term", "\"Homo sapiens\"[ORGN] AND \"RNA-Seq\"[STRA]"))
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
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let query = SearchQuery {
        organism: Some("Homo sapiens".into()),
        strategy: Some("RNA-Seq".into()),
        ..SearchQuery::new()
    };
    let rows = client.search(&query).await.unwrap();
    assert!(!rows.is_empty());
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sradb-core --test search_e2e 2>&1 | tail -10`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/search_e2e.rs
git commit -m "test(core): wiremock e2e for search (term construction + empty-query error)"
```

## Context for Task 5

The `query_param("term", ...)` matcher in `esearch_term_includes_orgn_and_stra_qualifiers` ensures the constructed term has the right shape — wiremock will return 404 (and the test fail) if the term doesn't match. This proves `build_term` is producing the expected output without separately exporting it for verification.

---

## Task 6: Final verification

- [ ] **Step 1: Build / fmt / clippy / test**

```bash
cargo build --workspace --all-targets 2>&1 | tail -3
cargo fmt --all -- --check 2>&1 | tail -2
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -3
```
Expected: all green; ≥73 tests (slice 4 baseline 65 + 6 unit + 3 e2e).

- [ ] **Step 2: Mark plan complete + commit + tag**

Add `✅` to all task headings in this plan, then:

```bash
git add docs/superpowers/plans/2026-04-26-sradb-rs-slice-5-search.md
git commit -m "docs(plan): mark all 6 slice-5 tasks complete"
git tag -a slice-5-search -m "Slice 5: SRA search via Entrez field qualifiers"
```

---

## Deferred

- ENA portal API search (slice 5b)
- GEO datasets search (slice 5c)
- Verbosity levels 0-3 (we always return full default rows)
- Date range filtering (`--publication-date`)
- `--mbases` size filtering
- Title filtering (`--title`)

## Definition of done

- `cargo test --workspace` ≥73 tests
- `sradb search --organism "Homo sapiens" --strategy RNA-Seq --max 3` returns 3 rows from live NCBI
- Wiremock e2e covers term construction + empty-query error
- `git tag slice-5-search` created
