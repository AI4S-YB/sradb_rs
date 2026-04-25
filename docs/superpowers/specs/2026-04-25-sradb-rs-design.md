# sradb-rs — Rust rewrite of pysradb

**Status:** Design approved, awaiting plan
**Date:** 2026-04-25
**Author:** brainstorm session

## Goal

Rewrite [pysradb](https://github.com/saketkc/pysradb) in Rust as a single Cargo workspace producing both a reusable async library (`sradb-core`) and a CLI binary (`sradb`). The rewrite targets full feature parity, including LLM-driven metadata enrichment, while reimagining the CLI surface with modern conventions and replacing pandas with light-weight typed structs.

## Non-goals

- Byte-for-byte CLI compatibility with pysradb (existing scripts will need to migrate).
- Python bindings (PyO3) in v1 — possible follow-on, not in scope here.
- Built-in DataFrame engine (polars). Typed structs only; users can convert downstream if needed.
- Local LLM inference. Enrichment is an HTTP call to an OpenAI-compatible endpoint.

## High-level decisions (from brainstorm)

| Decision | Choice | Rationale |
| --- | --- | --- |
| Scope | Full parity (metadata, search, download, conversions, identifier extraction, GEO matrix, enrichment) | User chose B |
| CLI compatibility | Reimagined, clap-native | User chose C |
| Data model | Typed structs + serde, no polars dep | User chose A |
| Distribution | Async lib + binary, no PyO3 in v1 | User chose B |
| Testing | Unit + recorded fixtures + golden snapshots | User chose C |
| Implementation strategy | Vertical slices, foundation first | Approach 2 |
| Enrichment backend | OpenAI-compatible chat completions API | User direction (replaces Ollama) |

## Architecture

### Workspace layout

```
sradb_rs/
├── Cargo.toml                      # workspace root
├── crates/
│   ├── sradb-core/                 # library: types, parsers, HTTP client
│   │   ├── src/
│   │   │   ├── lib.rs
│   │   │   ├── client.rs           # SraClient (async, the SRAweb equivalent)
│   │   │   ├── error.rs            # thiserror enum
│   │   │   ├── accession.rs        # parse/validate SRP/SRR/SRX/SRS/GSE/GSM/PMID/DOI/PMC
│   │   │   ├── http.rs             # reqwest wrapper: rate-limit, retry, api_key
│   │   │   ├── ncbi/
│   │   │   │   ├── esearch.rs
│   │   │   │   ├── esummary.rs
│   │   │   │   └── efetch.rs
│   │   │   ├── ena.rs              # ENA filereport + taxonomy REST
│   │   │   ├── geo/
│   │   │   │   ├── soft.rs         # GSM SOFT format parser
│   │   │   │   └── matrix.rs       # series_matrix.txt.gz parser
│   │   │   ├── parse/
│   │   │   │   ├── runinfo.rs      # efetch runinfo CSV
│   │   │   │   ├── experiment.rs   # EXPERIMENT_PACKAGE_SET XML
│   │   │   │   └── sample_attrs.rs # `key: value || key: value` parser
│   │   │   ├── model/              # public typed structs
│   │   │   ├── metadata.rs         # high-level metadata() orchestrator
│   │   │   ├── convert.rs          # accession-graph engine
│   │   │   ├── search.rs           # SraSearch / EnaSearch / GeoSearch
│   │   │   ├── download.rs         # async parallel HTTP/FTP downloader
│   │   │   └── enrich.rs           # OpenAI chat-completions client + ontology mapping
│   │   └── tests/                  # integration tests + golden snapshots
│   ├── sradb-cli/                  # binary: clap-driven CLI
│   │   └── src/
│   │       ├── main.rs
│   │       ├── cmd/                # one file per top-level subcommand
│   │       └── output.rs           # TSV / JSON / pretty table writers
│   └── sradb-fixtures/             # dev-only crate: fixture loaders for tests
├── tests/data/                     # captured XML/JSON fixtures (committed)
├── docs/superpowers/specs/
└── README.md
```

### Why three crates

- `sradb-core` is the reusable library; downstream Rust apps depend on it without pulling clap or terminal-UI deps.
- `sradb-cli` owns argv parsing, output formatting, and progress UI. CLI-specific tests do not need network.
- `sradb-fixtures` keeps fixture-loading helpers out of the public API but reusable across both crates' tests.

### External dependencies

**Core:** `tokio`, `reqwest` (rustls), `reqwest-middleware`, `reqwest-retry`, `serde`, `serde_json`, `quick-xml` (streaming XML), `csv`, `thiserror`, `governor` (rate limiting), `flate2`, `tracing`, `chrono`, `regex`, `suppaftp` (async FTP), `md5`.

**CLI adds:** `clap` (derive), `indicatif`, `tracing-subscriber`, `anyhow`.

**Tests add:** `wiremock`, `insta`, `tokio-test`, `proptest`, `criterion`.

### CLI surface

```
sradb metadata <ACCESSION>... [--detailed] [--format tsv|json|ndjson] [--enrich]
sradb convert <FROM> <TO> <ACCESSION>... [--detailed]
sradb search [--db sra|ena|geo] [--query ...] [--organism ...] [-v 0..3]
sradb download <ACCESSION>... [--out-dir ...] [-j N] [--source ena|sra|geo]
sradb geo matrix <GSE> [--parse-tsv] [--out-dir ...]
sradb id <PMID|DOI|PMC> [--type gse|srp|all]
```

`sradb convert <FROM> <TO>` collapses pysradb's 22 hand-written conversion subcommands into one form. Tab completion enumerates valid `<FROM> <TO>` pairs via clap value enums.

## Core types

### Accession

```rust
pub enum AccessionKind {
    Srp, Srx, Srs, Srr, Gse, Gsm, BioProject, Pmid, Doi, Pmc,
}

pub struct Accession {
    pub kind: AccessionKind,
    pub raw: String,
}

impl FromStr for Accession {
    type Err = ParseAccessionError;
    fn from_str(s: &str) -> Result<Self, Self::Err> { /* regex-based detect */ }
}
```

### Model structs

```rust
pub struct Study {
    pub accession: String,                // SRP...
    pub title: Option<String>,
    pub abstract_: Option<String>,
    pub bioproject: Option<String>,       // PRJNA...
    pub geo_accession: Option<String>,    // GSE... (filled when detailed)
    pub pmids: Vec<u64>,
}

pub struct Experiment {
    pub accession: String,                // SRX...
    pub title: Option<String>,
    pub study_accession: String,
    pub sample_accession: String,         // SRS...
    pub design_description: Option<String>,
    pub library: Library,
    pub platform: Platform,
    pub geo_accession: Option<String>,    // GSM...
}

pub struct Library {
    pub strategy: Option<String>,
    pub source: Option<String>,
    pub selection: Option<String>,
    pub layout: LibraryLayout,
    pub construction_protocol: Option<String>,
}

pub enum LibraryLayout {
    Single { length: Option<u32> },
    Paired { nominal_length: Option<u32>, nominal_sdev: Option<f32> },
    Unknown,
}

pub struct Platform {
    pub name: String,                     // ILLUMINA, OXFORD_NANOPORE, ...
    pub instrument_model: Option<String>,
}

pub struct Sample {
    pub accession: String,                // SRS...
    pub title: Option<String>,
    pub biosample: Option<String>,        // SAMN...
    pub organism_taxid: Option<u32>,
    pub organism_name: Option<String>,
    pub attributes: BTreeMap<String, String>,  // dynamic SAMPLE_ATTRIBUTES
}

pub struct Run {
    pub accession: String,                // SRR...
    pub experiment_accession: String,
    pub sample_accession: String,
    pub study_accession: String,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub total_size: Option<u64>,
    pub published: Option<chrono::DateTime<chrono::Utc>>,
    pub urls: RunUrls,
}

pub struct RunUrls {
    pub ena_fastq_http: Vec<String>,      // 0..2 entries
    pub ena_fastq_ftp: Vec<String>,
    pub ncbi_sra: Option<String>,
    pub s3: Option<String>,
    pub gs: Option<String>,
}

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

pub struct MetadataRow {
    pub run: Run,
    pub experiment: Experiment,
    pub sample: Sample,
    pub study: Study,
    pub enrichment: Option<Enrichment>,
}
```

`MetadataRow::to_record(&self, detailed: bool) -> Vec<(&'static str, String)>` produces ordered key-value pairs for TSV/CSV output. Dynamic sample attributes are appended after the fixed columns.

### Client API

```rust
pub struct SraClient { /* http, rate limiter, optional api_key, base URLs */ }

#[derive(Default)]
pub struct ClientConfig {
    pub api_key: Option<String>,         // NCBI api_key (env: NCBI_API_KEY)
    pub user_agent: Option<String>,
    pub timeout: Option<Duration>,
    pub max_retries: u32,                // default 5
    pub email: Option<String>,           // NCBI E-utils etiquette
    pub ncbi_base_url: Option<String>,   // override for testing
    pub ena_base_url: Option<String>,    // override for testing
}

impl SraClient {
    pub fn new() -> Self;
    pub fn with_config(cfg: ClientConfig) -> Self;

    pub async fn metadata(&self, acc: &Accession, opts: MetadataOpts)
        -> Result<Vec<MetadataRow>>;
    pub async fn metadata_many(&self, accs: &[Accession], opts: MetadataOpts)
        -> Vec<Result<Vec<MetadataRow>>>;

    pub async fn convert(&self, from: &Accession, to: AccessionKind)
        -> Result<Vec<Accession>>;
    pub async fn convert_detailed(&self, from: &Accession, to: AccessionKind)
        -> Result<Vec<MetadataRow>>;

    pub async fn search(&self, q: SearchQuery) -> Result<SearchResults>;
    pub async fn download(&self, plan: DownloadPlan, out_dir: &Path) -> Result<DownloadReport>;
    pub async fn geo_matrix(&self, gse: &str, out: &Path) -> Result<PathBuf>;
    pub fn parse_geo_matrix(path: &Path) -> Result<GeoMatrix>;

    pub async fn identifiers_from_pmid(&self, pmid: u64) -> Result<IdentifierSet>;
    pub async fn identifiers_from_doi(&self, doi: &str) -> Result<IdentifierSet>;
    pub async fn identifiers_from_pmc(&self, pmc: &str) -> Result<IdentifierSet>;

    pub async fn enrich(&self, rows: &mut [MetadataRow], cfg: EnrichConfig) -> Result<()>;
}
```

## Data flow

### `sradb metadata SRP000941 --detailed --enrich`

1. CLI: clap parses → `Accession::from_str("SRP000941")` → `AccessionKind::Srp`.
2. `SraClient::metadata(acc, MetadataOpts { detailed: true, enrich: true })`.
3. `ncbi::esearch(db="sra", term="SRP000941")` → `(WebEnv, query_key)`.
4. `ncbi::esummary(db="sra", WebEnv, retmax=500, paginated)` → `Vec<EsummaryDocSum>`. Per docsum: parse `<ExpXml>` + `<Runs>` XML → `Vec<(Experiment, Vec<Run>, Sample)>`.
5. If detailed:
   1. `ncbi::efetch(db="sra", WebEnv, retmode="runinfo")` → CSV; augment Run with `total_bases`, `published`, `total_size`.
   2. `ncbi::efetch(db="sra", WebEnv)` → EXPERIMENT_PACKAGE_SET XML; pull SAMPLE_ATTRIBUTES into `sample.attributes`; pull SRAFiles into `run.urls`.
   3. `ena::filereport(accession=run, fields=fastq_ftp,fastq_md5,fastq_bytes)` per run; fan-out via tokio with semaphore N=8 → `run.urls.ena_fastq_*`.
   4. `convert(srp, AccessionKind::Gse)` → `study.geo_accession`.
   5. `ncbi::esummary(db="gds", filtered to GSM)` → `experiment.geo_accession`.
6. If enrich: build prompt per row from `sample.attributes` + titles, fan-out to OpenAI chat completions with bounded semaphore (default 8), parse structured JSON → `Enrichment`. See "Enrichment" section.
7. Return `Vec<MetadataRow>`.
8. CLI: `output::write(rows, format, detailed)` → stdout.

### Conversions

Lookup table in `convert.rs`:

- `EsummarySraField`: one esummary against `db=sra`, project a field.
- `EsummaryGdsField`: one esummary against `db=gds`.
- `GdsThenSra`: GDS lookup yields an SRP, then SRA esummary.
- `RunInfoCsv`: efetch runinfo, project a column.

The 25+ pysradb conversion methods collapse to one `match (from.kind, to)` returning a strategy.

### Search

- `--db sra`: clap → `SraSearch::build_query()` → URL-encoded esearch term with field qualifiers (`SRR000001[ACCN] AND illumina[PLAT]`) → esummary pagination → typed `SearchHit`.
- `--db ena`: ENA portal API (`https://www.ebi.ac.uk/ena/portal/api/search`) — single TSV response, parse rows directly.
- `--db geo`: esearch + esummary against `db=gds`, identical pagination to SRA path.

### Download

- Resolve accession → list of run URLs (calls `metadata` internally if input is SRP/SRX/GSM/GSE).
- Plan: list of `(url, dest_path, expected_size, expected_md5)`.
- Worker pool of N tasks. Each: HEAD for content-length → GET with `Range: bytes=N-` resume → write to `.part` → md5 verify → atomic rename.
- FTP path uses `suppaftp` async crate; same resume + verify flow.
- Progress: `indicatif` MultiProgress, one bar per active download + global aggregate.

### Concurrency pools

Three distinct pools, all `tokio` + bounded semaphores:

- **NCBI eUtils:** `governor` rate-limiter at 3 rps (no key) / 10 rps (with key). All esearch/esummary/efetch share one limiter.
- **ENA filereport:** semaphore at 8 concurrent.
- **Downloads:** user-configurable `-j N` (default 4).

Why three pools: NCBI throttles aggressively; if ENA fan-out shared NCBI's bucket, a metadata call with 100 runs would stall on the wrong limiter.

## Enrichment (OpenAI-compatible)

```rust
pub struct EnrichConfig {
    pub api_key: String,                  // env: OPENAI_API_KEY
    pub base_url: String,                 // default https://api.openai.com, env: OPENAI_BASE_URL
    pub model: String,                    // default gpt-4o-mini
    pub temperature: f32,                 // default 0.0
    pub concurrency: usize,               // default 8
    pub max_retries: u32,                 // default 3
    pub timeout: Duration,                // default 60s
}
```

**Wire format:** `POST {base_url}/v1/chat/completions` with body:

```json
{
  "model": "gpt-4o-mini",
  "messages": [
    { "role": "system", "content": "Extract biological metadata fields from the provided sample text. Return null for fields not determinable." },
    { "role": "user", "content": "<concatenated sample_title, experiment_title, sample attributes>" }
  ],
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "metadata_extraction",
      "strict": true,
      "schema": { /* JSON Schema covering 9 fields, all nullable strings */ }
    }
  },
  "temperature": 0.0
}
```

**Why OpenAI-compatible (not Ollama):** the `OPENAI_BASE_URL` override means users can point at OpenAI, Azure OpenAI, Together, Groq, vLLM, llama.cpp's server, or even Ollama's OpenAI-compatible endpoint. One client, many backends.

**Ontology reference:** `ontology_reference.json` (copied from `pysradb/pysradb/ontology_reference.json` during slice 7) is embedded via `include_str!`. Per pysradb behavior, it is loaded only by callers who explicitly request it — the default enrichment path passes the LLM output through unchanged. No post-processing normalization is added in v1; that would be scope creep beyond pysradb parity.

## Error handling

```rust
#[derive(thiserror::Error, Debug)]
pub enum SradbError {
    #[error("invalid accession: {input} ({reason})")]
    InvalidAccession { input: String, reason: String },

    #[error("accession not found: {0}")]
    NotFound(String),

    #[error("conversion not supported: {from:?} → {to:?}")]
    UnsupportedConversion { from: AccessionKind, to: AccessionKind },

    #[error("HTTP error from {endpoint}: {source}")]
    Http { endpoint: &'static str, #[source] source: reqwest::Error },

    #[error("rate limited by {service} after {retries} retries")]
    RateLimited { service: &'static str, retries: u32 },

    #[error("response parse error at {endpoint}: {message}")]
    Parse { endpoint: &'static str, message: String },

    #[error("XML parse error in {context}: {source}")]
    Xml { context: &'static str, #[source] source: quick_xml::Error },

    #[error("enrichment failed: {message}")]
    Enrichment { message: String, #[source] source: Option<reqwest::Error> },

    #[error("download failed for {url}: {reason}")]
    Download { url: String, reason: String },

    #[error("checksum mismatch for {path}: expected {expected}, got {got}")]
    ChecksumMismatch { path: PathBuf, expected: String, got: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SradbError>;
```

### Retry policy

Lives in `http.rs`, not at call sites:

- Idempotent GETs: retry on 429, 500, 502, 503, 504, transient TCP.
- Exponential backoff with jitter: 0.5s → 1s → 2s → 4s → 8s, max 5 attempts, capped 30s total.
- 429 with `Retry-After` header → honor the header.
- Other 4xx → no retry, immediate `NotFound` or `Http` error.
- Backed by `reqwest-middleware` + `reqwest-retry`.

### Partial failure

- `metadata_many([SRP1, SRP2_BAD, SRP3])` → `Vec<Result<Vec<MetadataRow>>>`. CLI prints failed accessions to stderr, prints rows from the rest, exits non-zero only if all failed.
- ENA fastq fan-out failure on 1 of 100 runs: log warning, leave `urls.ena_fastq_*` empty for that run, do not fail the whole call.
- Enrichment failure for a row: leave `enrichment: None`, log, continue.

### Logging

Library uses `tracing`, never prints to stdout/stderr. CLI installs `tracing-subscriber`. `-v` raises level: default WARN → `-v` INFO → `-vv` DEBUG → `-vvv` TRACE. HTTP request/response (without bodies) at DEBUG; bodies at TRACE.

### CLI exit codes

- `0` — success (full or partial-with-output)
- `1` — all accessions failed / unrecoverable error
- `2` — usage error (clap-handled)
- `3` — enrichment requested but `OPENAI_API_KEY` missing or auth failed
- `4` — checksum verification failed during download

### Cancellation

All async paths take tokio cooperative cancellation. Ctrl-C in the CLI cancels in-flight downloads; partial `.part` files preserved (resumable on next run).

## Testing

### Layer 1 — Unit tests (in-module)

Pure functions, no I/O.

- `accession.rs`: parse all 10 accession types, reject malformed. Table-driven.
- `parse/sample_attrs.rs`: pipe-delimited `key: value || key: value`, including values containing colons, quoted values, empty fields.
- `parse/runinfo.rs`: CSV → `Run`, missing fields → `None`.
- `parse/experiment.rs`: EXPERIMENT_PACKAGE_SET XML → typed structs, one test per pysradb fixture.
- `convert.rs`: lookup-table sanity — every `(from, to)` pair has a strategy, no panics on `(X, X)`.

### Layer 2 — Recorded-fixture tests

`crates/sradb-core/tests/`. `wiremock` stub server on random port; `SraClient` constructed with custom base URL. Fixtures in `tests/data/`:

```
tests/data/
├── ncbi/
│   ├── esearch_SRP016501.json
│   ├── esummary_SRP016501.xml
│   ├── efetch_runinfo_SRP016501.csv
│   ├── efetch_experiment_SRP016501.xml
│   └── ... (~30 captured calls)
├── ena/
│   └── filereport_SRR057511.tsv
├── geo/
│   └── GSE56924_series_matrix.txt.gz
└── openai/
    └── chat_completion_metadata_extraction.json
```

For each test from `test_sraweb.py` (24 cases from the audit), one Rust test with the matching fixture(s).

Fixtures captured once via `tools/capture-fixtures` binary that hits real endpoints. Re-runnable when API shape drifts. Committed. Total <2MB compressed.

### Layer 3 — Golden snapshots (`insta`)

For each `metadata` test case, snapshot parsed `Vec<MetadataRow>` as JSON in `tests/snapshots/`. `cargo insta review` for human accept/reject.

### Live integration tests

`#[cfg(feature = "live")]`-gated. Same names as Layer 2, hit real endpoints. Run manually before releases. `cargo test --features live -- --test-threads=1`.

### Property tests (`proptest`)

- Accession parser: round-trip `Display`/`FromStr`.
- TSV/JSON output: any `MetadataRow` round-trips through serialize/deserialize unchanged.

### CI matrix

- `cargo test` (no features): Layers 1, 2, 3 + property tests. Hermetic, target <30s.
- `cargo clippy --all-targets -- -D warnings`
- `cargo fmt --check`
- Live tests on `release/*` branches and manual `workflow_dispatch` only.

### Bench targets (`criterion`, `benches/`)

- `parse_experiment_xml` — large EXPERIMENT_PACKAGE_SET (1000 experiments).
- `parse_runinfo_csv` — 10k rows.
- `parse_sample_attrs` — 1000 attribute strings.

Reference numbers in `BENCHES.md`. Not blocking in CI.

## Implementation strategy (vertical slices)

Per the brainstorm decision, ship narrow end-to-end features one at a time. Each slice lands as a working `sradb <subcommand>`:

1. **Foundation** — workspace, `sradb-core` skeleton, `Accession` type, async HTTP client with rate limit + retry, error type, golden-test harness, fixture capture tool.
2. **`sradb metadata <SRP>`** — full vertical: efetch → parse XML → typed structs → TSV/JSON output. Default columns first, then `--detailed`.
3. **Accession conversions** — `sradb convert <FROM> <TO>` covering all 25+ pysradb conversions through one engine.
4. **`sradb search`** — SRA, ENA, GEO backends.
5. **`sradb download`** — parallel HTTP/FTP with resume + checksum.
6. **`sradb geo matrix`** — series_matrix.txt.gz parser.
7. **`sradb metadata --enrich`** — OpenAI-compatible chat completions client wired into the metadata orchestrator.
8. **`sradb id`** — PMID/DOI/PMC identifier extraction.
9. **CLI polish, completions, README, release.**

Each slice is a candidate for parallel subagent execution after slice 1 lands.

## Open questions / explicit deferrals

- **PyO3 bindings:** out of scope for v1, possible follow-on once the core is stable.
- **Polars feature flag:** out of scope for v1; the `Vec<MetadataRow>` → DataFrame helper can be added later behind `features = ["polars"]` without breaking the core API.
- **Async-trait for backends:** if needed for testing, swap concrete `SraClient` for a trait. Not v1.
- **`bsc` Bioscience search backend** (notebook 11 in pysradb): unclear maturity — defer to slice 4 review.

## Success criteria

- All 24 test cases from `test_sraweb.py` pass against captured fixtures (Layer 2 green).
- Golden snapshots stable across `cargo test` runs (Layer 3 green).
- `sradb metadata SRP000941 --detailed` returns row count and column set matching pysradb against the same accession (live spot-check).
- CI matrix green: build, test, clippy, fmt.
- Release artifact: `cargo install sradb-cli` produces a working binary on Linux/macOS.
