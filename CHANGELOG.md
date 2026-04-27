# Changelog

All notable changes to this project. Format follows [Keep a Changelog](https://keepachangelog.com/) loosely.

## 0.1.3 - 2026-04-27

### Fixed
- Raw downloads now retry interrupted response bodies using the existing `.part` file and HTTP `Range`, so transient proxy/server disconnects can resume instead of failing the file immediately.

## 0.1.2 - 2026-04-27

### Fixed
- Raw file downloads now force HTTP/1.1 and `Accept-Encoding: identity`, matching `wget` more closely and avoiding HTTP/2/proxy body stream failures reported as `error decoding response body`.

## 0.1.1 - 2026-04-27

### Fixed
- `sradb download` now defaults to NCBI SRA / SRA Lite URLs and uses ENA/EBI FASTQ URLs only when `--source ena` is provided.
- Raw file downloads disable automatic gzip response decoding, avoiding corrupt or failed `.fastq.gz` transfers from servers that mark compressed files as content-encoded.

## 0.1.0 - 2026-04-27

Initial Rust port of pysradb.

### Slice 10 — Polish
- `indicatif` progress bar for `sradb download`.
- README rewrite covering full CLI surface and env vars.
- CHANGELOG.

### Slice 9 — Identifier extraction
- `sradb id <PMID|DOI|PMC>` extracts GSE / GSM / SRP / PRJNA accessions from PMC fulltext.
- NCBI elink wrapper (pubmed → pmc).
- `IdentifierSet` typed struct with `--json` output.

### Slice 8 — LLM enrichment
- `sradb metadata --enrich` populates 9 ontology fields (organ, tissue, anatomical_system, cell_type, disease, sex, development_stage, assay, organism) via an OpenAI-compatible chat completions endpoint with strict JSON schema.
- Configurable via `OPENAI_API_KEY`, `OPENAI_BASE_URL`, `OPENAI_MODEL`.
- Semaphore-bounded concurrent fan-out (default 8). Per-row failures isolated.

### Slice 7 — GEO Series Matrix
- `sradb geo matrix <GSE> [--out-dir DIR] [--parse-tsv]`.
- URL builder handles the `GSE{head}nnn` shard pattern.
- Gzipped TSV parser extracts `!Series_*` / `!Sample_*` metadata + the data table.

### Slice 6 — Download
- `sradb download <ACC>... -j N --out-dir DIR`.
- Parallel HTTP downloader with `Range`-based resume and atomic rename via `.part` files.
- Resolves accessions via slice-3 detailed metadata to obtain ENA fastq URLs.

### Slice 5 — Search
- `sradb search` with `--query / --organism / --strategy / --source / --selection / --layout / --platform / --max`.
- Builds Entrez query terms from typed `SearchQuery`, reuses the metadata orchestrator.

### Slice 4 — Accession conversion
- `sradb convert <FROM> <TO> <ACCESSION>...` covers all 21+ pysradb conversion pairs.
- Two strategies: `ProjectFromMetadata` (reuses metadata orchestrator) and `GdsLookup` (db=gds esummary). Plus `Chain` for two-leg conversions.
- 25-line strategy lookup table replaces 25+ pysradb methods.

### Slice 3 — Detailed metadata
- `sradb metadata --detailed` adds:
  - Per-sample SAMPLE_ATTRIBUTES (dynamic columns in TSV output).
  - NCBI / S3 / GCP download URLs from `<SRAFiles>/<Alternatives>`.
  - ENA fastq URLs (HTTP + FTP) via `/portal/api/filereport`, fan-out concurrency = 8.
  - Refined `total_bases` / `total_size` / `published` from efetch runinfo CSV.

### Slice 2 — Default metadata
- `sradb metadata <ACCESSION>` against NCBI esearch + esummary.
- Typed model structs: `Run`, `Experiment`, `Sample`, `Study`, `Library`, `Platform`, `MetadataRow`.
- TSV / JSON / NDJSON output writers.

### Slice 1 — Foundation
- Workspace, async HTTP client (rate-limit + retry middleware), `Accession` parser, `SradbError`, fixtures crate, CI workflow.

## Notes

- MSRV 1.80 (`std::sync::LazyLock`).
- Tests: 100 across 16 suites at the end of slice 9.
- Dependencies: `tokio`, `reqwest` (rustls), `quick-xml`, `serde`, `csv`, `flate2`, `governor`, `clap`, `indicatif`.
