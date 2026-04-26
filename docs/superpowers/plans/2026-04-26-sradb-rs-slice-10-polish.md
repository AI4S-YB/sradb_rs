# sradb-rs Slice 10: Release Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Release-readiness polish — `indicatif` progress bars on `sradb download`, README rewrite covering all subcommands and env-vars, CHANGELOG with the slice history, and final CI gate.

**Architecture:** Three lightweight tasks. No new modules in `sradb-core`; only the CLI gets a progress-UI dependency.

**Tech Stack:** existing `indicatif = "0.17"` (already a workspace dep from slice 1).

**Reference:** Slices 1-9 complete.

---

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-cli/Cargo.toml` | (modify) add `indicatif.workspace = true` |
| `crates/sradb-cli/src/cmd/download.rs` | (modify) wrap `download_plan` calls with a `ProgressBar` |
| `README.md` | (rewrite) full CLI surface, env vars, install instructions |
| `CHANGELOG.md` | (create) slice-by-slice history |

---

## Task 1: indicatif progress bars in download ✅

**Files:**
- Modify: `crates/sradb-cli/Cargo.toml`
- Modify: `crates/sradb-cli/src/cmd/download.rs`

The slice-6 plan deferred `indicatif`. This task wires it up: a single `ProgressBar` shows total file count and per-file completion.

- [ ] **Step 1: Add indicatif to sradb-cli deps**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/Cargo.toml`. In `[dependencies]`, add `indicatif.workspace = true` (it's already in workspace deps from slice 1).

- [ ] **Step 2: Wrap download with a progress bar**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/download.rs`. Replace the body after the `let plan = DownloadPlan { items };` line with:

```rust
    let plan = DownloadPlan { items };
    let total = plan.items.len() as u64;
    let bar = indicatif::ProgressBar::new(total);
    bar.set_style(
        indicatif::ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} files {msg}",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    bar.set_message(format!("parallelism={}", args.parallelism));

    // Execute the download. The current `download_plan` signature returns
    // a `DownloadReport` synchronously after all futures complete; for
    // per-file progress, we'd need to refactor to take a callback.
    // Slice 10 keeps the simple version: bar finishes after the batch.
    let report = client.download(&plan, args.parallelism).await;
    bar.set_position(total);
    bar.finish_with_message(format!(
        "downloaded={}, skipped={}, failed={}",
        report.completed, report.skipped, report.failed
    ));

    if report.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
```

(Replace the previous `println!`-based reporting; the progress bar handles the final message.)

- [ ] **Step 3: Build**

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 4: Smoke**

Run: `cargo run -p sradb-cli --quiet -- download --help 2>&1 | tail -10`
Expected: same help as before; flags unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-cli/Cargo.toml crates/sradb-cli/src/cmd/download.rs
git commit -m "feat(cli): indicatif progress bar for sradb download"
```

---

## Task 2: README rewrite ✅

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Write README**

Read the current `/home/xzg/project/sradb_rs/README.md`. Replace its contents with:

```markdown
# sradb-rs

A Rust port of [pysradb](https://github.com/saketkc/pysradb) — query and download NGS metadata and data from NCBI SRA, ENA, and GEO.

## Status

Slices 1–10 complete. Feature-equivalent with pysradb's CLI for the most-used workflows.

## Installation

```bash
cargo install --path crates/sradb-cli
```

Requires Rust 1.80+.

## CLI surface

```bash
sradb metadata <ACCESSION>... [--detailed] [--enrich] [--format tsv|json|ndjson]
sradb convert <FROM> <TO> <ACCESSION>...
sradb search [--query ...] [--organism ...] [--strategy ...] [--platform ...]
sradb download <ACCESSION>... [--out-dir DIR] [-j N]
sradb geo matrix <GSE> [--out-dir DIR] [--parse-tsv]
sradb id <PMID|DOI|PMC> [--json]
sradb info
```

### Examples

Fetch metadata as TSV:
```bash
sradb metadata SRP174132 --format tsv
```

Get full detail with sample attributes and ENA fastq URLs:
```bash
sradb metadata SRP174132 --detailed --format json
```

Enrich with LLM-extracted ontology fields (organ / tissue / cell_type / etc.):
```bash
export OPENAI_API_KEY=sk-...
sradb metadata SRP174132 --detailed --enrich --format json
```

Convert between accession kinds:
```bash
sradb convert srp srx SRP174132     # → 10 SRX accessions
sradb convert gse srp GSE56924      # → SRP041298
sradb convert gsm srp GSM1371490
```

Search SRA:
```bash
sradb search --organism "Homo sapiens" --strategy RNA-Seq --max 10 --format json
```

Download ENA fastq files in parallel:
```bash
sradb download SRP174132 --out-dir ./fastq -j 4
```

Download a GEO Series Matrix:
```bash
sradb geo matrix GSE56924 --out-dir ./geo --parse-tsv
```

Extract identifiers from a PMID:
```bash
sradb id 39528918 --json
```

## Configuration

Environment variables:

| Variable | Purpose |
| --- | --- |
| `NCBI_API_KEY` | Raises NCBI rate limit from 3 rps to 10 rps |
| `NCBI_EMAIL` | Recommended by NCBI eUtils etiquette |
| `OPENAI_API_KEY` | Required for `--enrich` |
| `OPENAI_BASE_URL` | Override (default `https://api.openai.com`) — works with any OpenAI-compatible endpoint (Azure, Together, vLLM, llama.cpp server, Ollama's /v1 endpoint) |
| `OPENAI_MODEL` | Override (default `gpt-4o-mini`) |

## Architecture

Cargo workspace with three crates:

- `crates/sradb-core/` — async library; types, HTTP client, parsers, orchestrator
- `crates/sradb-cli/` — `sradb` binary (clap-based CLI)
- `crates/sradb-fixtures/` — dev-only test helpers

Plus a dev-tool `tools/capture-fixtures/` for capturing real-API responses for offline tests.

## Development

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

For development, the original Python `pysradb/` is kept in tree as reference (gitignored).

## Testing strategy

- **Unit tests** — pure functions, no I/O.
- **Recorded-fixture tests** — wiremock stub server replays captured XML/JSON/TSV from `tests/data/`.
- **Live tests** — gated behind `--features live`, run manually before releases.
- **Property tests** — `proptest` for the accession parser round-trip.

## License

MIT.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: rewrite README covering full CLI surface (slices 1–10)"
```

---

## Task 3: CHANGELOG ✅

**Files:**
- Create: `CHANGELOG.md`

- [ ] **Step 1: Write CHANGELOG**

Create `/home/xzg/project/sradb_rs/CHANGELOG.md`:

```markdown
# Changelog

All notable changes to this project. Format follows [Keep a Changelog](https://keepachangelog.com/) loosely.

## Unreleased / 0.1.0

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
- Tests: ~98 across 16 suites at the end of slice 9.
- Dependencies: `tokio`, `reqwest` (rustls), `quick-xml`, `serde`, `csv`, `flate2`, `governor`, `clap`, `indicatif`.
```

- [ ] **Step 2: Commit**

```bash
git add CHANGELOG.md
git commit -m "docs: CHANGELOG covering slices 1–10"
```

---

## Task 4: Final verification + tag ✅

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
git add docs/superpowers/plans/2026-04-26-sradb-rs-slice-10-polish.md
git commit -m "docs(plan): mark all 4 slice-10 tasks complete"
git tag -a slice-10-polish -m "Slice 10: release polish (progress bars, README, CHANGELOG)"
```

- [ ] **Step 3: Verify the full tag set**

```bash
git tag --list 'slice-*' --sort=v:refname
```

Expected:
```
slice-1-foundation
slice-2-metadata
slice-3-detailed
slice-4-convert
slice-5-search
slice-6-download
slice-7-geo-matrix
slice-8-enrich
slice-9-id-extraction
slice-10-polish
```

---

## Definition of done

- `cargo test --workspace` green
- `cargo install --path crates/sradb-cli` produces a working `sradb` binary
- README covers all 7 subcommands + env vars + dev workflow
- CHANGELOG covers all 10 slices
- All 10 slice tags exist
