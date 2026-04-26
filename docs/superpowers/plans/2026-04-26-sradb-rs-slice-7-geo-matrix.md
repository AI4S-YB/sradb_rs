# sradb-rs Slice 7: GEO Matrix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land `sradb geo matrix <GSE>` — download `<GSE>_series_matrix.txt.gz` from GEO's FTP and optionally parse the gzipped TSV section into a typed structure.

**Architecture:** A `geo/matrix.rs` module under sradb-core. Download happens via plain HTTPS to `https://ftp.ncbi.nlm.nih.gov/geo/series/<prefix>nnn/<GSE>/matrix/<GSE>_series_matrix.txt.gz`. Parsing splits on `!series_matrix_table_begin` / `!series_matrix_table_end` markers, decompresses with `flate2`, and extracts the inner TSV.

**Tech Stack:** `flate2` (gzip), `reqwest` for the download, slice-6 download semantics where applicable.

**Reference:** Slices 1-6 complete. The pysradb `geoweb.py` module is the parity target; we keep the same URL pattern and parser shape.

---

## Background: GEO matrix file format

A typical `<GSE>_series_matrix.txt.gz` (uncompressed) has three sections:

```
!Series_title       "..."
!Series_summary     "..."
!Series_overall_design  "..."
... (many !Series_*  and !Sample_*  metadata lines)

!series_matrix_table_begin
"ID_REF"    "GSM..."    "GSM..."    ...
"PROBE_1"   12.34       9.87        ...
...
!series_matrix_table_end
```

We extract:
- `series_metadata`: a `BTreeMap<String, String>` of leading `!Series_*` / `!Sample_*` lines (deduplicated by key — values for repeated keys joined by `\n`).
- `data_table`: the TSV between the two markers, including the header row.

URL pattern (NCBI FTP over HTTPS):
```
https://ftp.ncbi.nlm.nih.gov/geo/series/<prefix>nnn/<GSE>/matrix/<GSE>_series_matrix.txt.gz
```
where `<prefix>` is the GSE number with the last 3 digits replaced by `nnn`. Examples:
- GSE56924 → `GSE56nnn`
- GSE253406 → `GSE253nnn`
- GSE1 → `GSEnnn` (special edge case)

---

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/geo/mod.rs` | Module root |
| `crates/sradb-core/src/geo/matrix.rs` | URL builder, download, parser |
| `crates/sradb-core/src/lib.rs` | (modify) `pub mod geo;` |
| `crates/sradb-core/src/client.rs` | (modify) `SraClient::geo_matrix` |
| `crates/sradb-cli/src/cmd/geo.rs` | CLI handler |
| `crates/sradb-cli/src/cmd.rs` | (modify) `pub mod geo;` |
| `crates/sradb-cli/src/main.rs` | (modify) register `Geo` subcommand |
| `crates/sradb-core/Cargo.toml` | (modify) add `flate2.workspace = true` (or new dep) |
| `crates/sradb-core/tests/geo_matrix_e2e.rs` | Tests for URL builder + parser |

---

## Task 1: GEO matrix URL builder + types

**Files:**
- Create: `crates/sradb-core/src/geo/mod.rs`
- Create: `crates/sradb-core/src/geo/matrix.rs`
- Modify: `crates/sradb-core/src/lib.rs`

- [ ] **Step 1: Update lib.rs**

Add `pub mod geo;` (alphabetical, between `error` and `http`):

```rust
pub mod accession;
pub mod client;
pub mod convert;
pub mod download;
pub mod ena;
pub mod error;
pub mod geo;
pub mod http;
pub mod metadata;
pub mod model;
pub mod ncbi;
pub mod parse;
pub mod search;
```

- [ ] **Step 2: Create geo/mod.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/geo/mod.rs`:

```rust
//! GEO (Gene Expression Omnibus) helpers.

pub mod matrix;
```

- [ ] **Step 3: Create geo/matrix.rs (URL builder + types only)**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/geo/matrix.rs`:

```rust
//! GEO Series Matrix: URL builder, download, gzipped TSV parser.

use std::collections::BTreeMap;

use crate::error::{Result, SradbError};

/// Parsed `<GSE>_series_matrix.txt` content.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeoMatrix {
    /// Lines starting with `!Series_*` or `!Sample_*`. Repeated keys are joined with `\n`.
    pub series_metadata: BTreeMap<String, String>,
    /// The TSV table between `!series_matrix_table_begin` and `!series_matrix_table_end`,
    /// including the header row.
    pub data_table: String,
}

const NCBI_GEO_FTP_HTTPS: &str = "https://ftp.ncbi.nlm.nih.gov/geo/series";

/// Compute the canonical GEO matrix URL for a `GSE<digits>` accession.
///
/// Returns an error if the accession isn't `GSE<digits>`.
pub fn matrix_url(gse: &str) -> Result<String> {
    let acc = gse.trim();
    if !acc.starts_with("GSE") || acc.len() < 4 {
        return Err(SradbError::InvalidAccession {
            input: gse.to_owned(),
            reason: "expected GSE<digits>".into(),
        });
    }
    let digits_part = &acc[3..];
    if !digits_part.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SradbError::InvalidAccession {
            input: gse.to_owned(),
            reason: "non-digit characters after GSE prefix".into(),
        });
    }
    // Replace the last 3 digits with "nnn" to form the FTP shard.
    let prefix = if digits_part.len() <= 3 {
        "GSEnnn".to_owned()
    } else {
        let head = &digits_part[..digits_part.len() - 3];
        format!("GSE{head}nnn")
    };
    Ok(format!(
        "{NCBI_GEO_FTP_HTTPS}/{prefix}/{acc}/matrix/{acc}_series_matrix.txt.gz"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_url_typical() {
        assert_eq!(
            matrix_url("GSE56924").unwrap(),
            "https://ftp.ncbi.nlm.nih.gov/geo/series/GSE56nnn/GSE56924/matrix/GSE56924_series_matrix.txt.gz"
        );
        assert_eq!(
            matrix_url("GSE253406").unwrap(),
            "https://ftp.ncbi.nlm.nih.gov/geo/series/GSE253nnn/GSE253406/matrix/GSE253406_series_matrix.txt.gz"
        );
    }

    #[test]
    fn matrix_url_short_accession() {
        assert_eq!(
            matrix_url("GSE1").unwrap(),
            "https://ftp.ncbi.nlm.nih.gov/geo/series/GSEnnn/GSE1/matrix/GSE1_series_matrix.txt.gz"
        );
    }

    #[test]
    fn matrix_url_invalid() {
        assert!(matrix_url("SRP174132").is_err());
        assert!(matrix_url("GSE").is_err());
        assert!(matrix_url("GSE12abc").is_err());
    }
}
```

- [ ] **Step 4: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test -p sradb-core --lib geo 2>&1 | tail -5`
Expected: 3 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/geo
git commit -m "feat(geo): GeoMatrix type + matrix_url builder for GSE accessions"
```

---

## Task 2: GEO matrix parser

**Files:**
- Modify: `crates/sradb-core/src/geo/matrix.rs`
- Modify: `crates/sradb-core/Cargo.toml` (add `flate2.workspace = true` if not present)

- [ ] **Step 1: Verify flate2 in deps**

Read `crates/sradb-core/Cargo.toml`. `flate2` is already in workspace deps (added in slice 1). Check if it's listed in this crate's `[dependencies]`. If not, add:

```toml
flate2 = { workspace = false, version = "1" }
```

Wait — read the workspace Cargo.toml first to confirm `flate2` is in `[workspace.dependencies]`. Slice 1 listed it. If yes, simply add `flate2.workspace = true` to `crates/sradb-core/Cargo.toml`.

If `flate2` is NOT in workspace deps, add it to BOTH:
- workspace `[workspace.dependencies]`: `flate2 = "1"`
- `crates/sradb-core/Cargo.toml [dependencies]`: `flate2.workspace = true`

- [ ] **Step 2: Append parse_matrix function**

Append to `crates/sradb-core/src/geo/matrix.rs`:

```rust

use std::io::Read;

/// Parse a (decompressed) series_matrix.txt body into a `GeoMatrix`.
pub fn parse_matrix(text: &str) -> Result<GeoMatrix> {
    let mut series_metadata: BTreeMap<String, String> = BTreeMap::new();
    let mut data_lines: Vec<&str> = Vec::new();
    let mut in_table = false;

    for line in text.lines() {
        if line.starts_with("!series_matrix_table_begin") {
            in_table = true;
            continue;
        }
        if line.starts_with("!series_matrix_table_end") {
            in_table = false;
            continue;
        }
        if in_table {
            data_lines.push(line);
            continue;
        }
        if let Some(rest) = line.strip_prefix('!') {
            if let Some((key, value)) = rest.split_once('\t') {
                let key = key.trim().to_owned();
                let value = value.trim_matches('"').to_owned();
                series_metadata
                    .entry(key)
                    .and_modify(|existing| {
                        existing.push('\n');
                        existing.push_str(&value);
                    })
                    .or_insert(value);
            }
        }
    }

    Ok(GeoMatrix {
        series_metadata,
        data_table: data_lines.join("\n"),
    })
}

/// Decompress gzipped bytes and parse via `parse_matrix`.
pub fn parse_matrix_gz(bytes: &[u8]) -> Result<GeoMatrix> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut text = String::new();
    decoder.read_to_string(&mut text).map_err(SradbError::Io)?;
    parse_matrix(&text)
}
```

Append to the test module:

```rust

    const SAMPLE_MATRIX: &str = "!Series_title\t\"Test study\"\n\
!Series_summary\t\"Line 1\"\n\
!Series_summary\t\"Line 2\"\n\
!Sample_title\t\"sample 1\"\t\"sample 2\"\n\
!series_matrix_table_begin\n\
\"ID_REF\"\t\"GSM1\"\t\"GSM2\"\n\
\"PROBE_A\"\t1.0\t2.0\n\
\"PROBE_B\"\t3.0\t4.0\n\
!series_matrix_table_end\n\
";

    #[test]
    fn parses_metadata_and_table() {
        let m = parse_matrix(SAMPLE_MATRIX).unwrap();
        assert_eq!(m.series_metadata.get("Series_title").map(String::as_str), Some("Test study"));
        // Repeated keys are joined with newline.
        let summary = m.series_metadata.get("Series_summary").unwrap();
        assert!(summary.contains("Line 1"));
        assert!(summary.contains("Line 2"));
        assert!(m.data_table.contains("ID_REF"));
        assert!(m.data_table.contains("PROBE_A"));
        assert_eq!(m.data_table.lines().count(), 3); // header + 2 rows
    }

    #[test]
    fn parses_empty_body() {
        let m = parse_matrix("").unwrap();
        assert!(m.series_metadata.is_empty());
        assert_eq!(m.data_table, "");
    }

    #[test]
    fn parses_round_trip_through_gzip() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(SAMPLE_MATRIX.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();
        let m = parse_matrix_gz(&gz).unwrap();
        assert!(m.data_table.contains("PROBE_A"));
    }
```

- [ ] **Step 3: Build + tests**

Run: `cargo test -p sradb-core --lib geo 2>&1 | tail -5`
Expected: 6 tests PASS (3 URL + 3 parse).

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/geo/matrix.rs crates/sradb-core/Cargo.toml
git commit -m "feat(geo): parse_matrix + parse_matrix_gz (series + sample metadata + TSV table)"
```

---

## Task 3: SraClient::geo_matrix download + CLI

**Files:**
- Modify: `crates/sradb-core/src/client.rs`
- Create: `crates/sradb-cli/src/cmd/geo.rs`
- Modify: `crates/sradb-cli/src/cmd.rs`
- Modify: `crates/sradb-cli/src/main.rs`

- [ ] **Step 1: Append SraClient::geo_matrix**

Inside `impl SraClient`, after `download` (or after `search`):

```rust

    /// Download a GEO Series Matrix `.txt.gz` for a GSE accession.
    /// Returns the gzipped bytes; use `geo::matrix::parse_matrix_gz` to decode.
    pub async fn geo_matrix_download(&self, gse: &str) -> Result<Vec<u8>> {
        let url = crate::geo::matrix::matrix_url(gse)?;
        let raw = reqwest::Client::builder()
            .timeout(self.cfg.timeout)
            .user_agent(format!("sradb-rs/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client build");
        let resp = raw.get(&url).send().await.map_err(|source| crate::error::SradbError::Http {
            endpoint: "geo_matrix",
            source,
        })?;
        if !resp.status().is_success() {
            return Err(crate::error::SradbError::Download {
                url,
                reason: format!("status {}", resp.status()),
            });
        }
        let bytes = resp.bytes().await.map_err(|source| crate::error::SradbError::Http {
            endpoint: "geo_matrix",
            source,
        })?;
        Ok(bytes.to_vec())
    }
```

- [ ] **Step 2: Update cmd.rs**

```rust
//! Subcommand handlers.

pub mod convert;
pub mod download;
pub mod geo;
pub mod metadata;
pub mod search;
```

- [ ] **Step 3: Create cmd/geo.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/geo.rs`:

```rust
//! `sradb geo matrix <GSE> [--out-dir DIR] [--parse-tsv]` handler.

use std::path::PathBuf;

use clap::{Args, Subcommand};
use sradb_core::geo::matrix::parse_matrix_gz;
use sradb_core::{ClientConfig, SraClient};

#[derive(Args, Debug)]
pub struct GeoArgs {
    #[command(subcommand)]
    pub cmd: GeoCmd,
}

#[derive(Subcommand, Debug)]
pub enum GeoCmd {
    /// Download a GEO Series Matrix file for a GSE accession.
    Matrix {
        /// GSE accession (e.g. `GSE56924`).
        gse: String,

        /// Output directory.
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,

        /// Also write the parsed TSV next to the .gz (header + table only).
        #[arg(long, default_value_t = false)]
        parse_tsv: bool,
    },
}

pub async fn run(args: GeoArgs) -> anyhow::Result<()> {
    match args.cmd {
        GeoCmd::Matrix { gse, out_dir, parse_tsv } => matrix_run(&gse, &out_dir, parse_tsv).await,
    }
}

async fn matrix_run(gse: &str, out_dir: &PathBuf, parse_tsv: bool) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let bytes = client.geo_matrix_download(gse).await?;

    std::fs::create_dir_all(out_dir)?;
    let gz_path = out_dir.join(format!("{gse}_series_matrix.txt.gz"));
    std::fs::write(&gz_path, &bytes)?;
    println!("wrote {} ({} bytes)", gz_path.display(), bytes.len());

    if parse_tsv {
        let parsed = parse_matrix_gz(&bytes)?;
        let tsv_path = out_dir.join(format!("{gse}_series_matrix.tsv"));
        std::fs::write(&tsv_path, parsed.data_table.as_bytes())?;
        println!(
            "wrote {} ({} bytes; {} metadata keys)",
            tsv_path.display(),
            parsed.data_table.len(),
            parsed.series_metadata.len()
        );
    }
    Ok(())
}
```

- [ ] **Step 4: Update main.rs**

In the `Cmd` enum (after `Download`):

```rust
    /// GEO helpers (matrix download/parse).
    Geo(cmd::geo::GeoArgs),
```

In the match block:

```rust
        Some(Cmd::Geo(args)) => cmd::geo::run(args).await,
```

- [ ] **Step 5: Build + smoke help**

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: PASS.

Run: `cargo run -p sradb-cli --quiet -- geo matrix --help 2>&1 | tail -10`
Expected: clap help.

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-core/src/client.rs crates/sradb-cli/src/cmd.rs crates/sradb-cli/src/cmd/geo.rs crates/sradb-cli/src/main.rs
git commit -m "feat(cli): sradb geo matrix <GSE> [--out-dir ...] [--parse-tsv]"
```

---

## Task 4: Final verification

- [ ] **Step 1: All gates**

```bash
cargo build --workspace --all-targets 2>&1 | tail -3
cargo fmt --all -- --check 2>&1 | tail -2
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -3
```
Expected: green; ≥82 tests.

- [ ] **Step 2: Live smoke**

Run: `cargo run -p sradb-cli --quiet -- geo matrix GSE56924 --out-dir /tmp/sradb_geo --parse-tsv 2>&1 | head -3`
Expected: writes the .gz file (typically 10-200 KB) and the parsed .tsv.

- [ ] **Step 3: Mark + tag**

```bash
git tag -a slice-7-geo-matrix -m "Slice 7: GEO Series Matrix download + parse"
```

---

## Deferred

- Download supplementary `geo/series/<GSE>/suppl/...` files (pysradb's `download_geo_files`)
- Parse the TSV `data_table` into typed `Vec<Vec<f64>>` (slice 7b — needs more thought on missing-value handling)
- Streaming-decompression path for very large matrices

## Definition of done

- `cargo test --workspace` ≥82 tests
- `sradb geo matrix GSE56924 --out-dir /tmp/sradb_geo --parse-tsv` writes both `.gz` and parsed `.tsv`
- 3 URL-builder tests + 3 parser tests in `parse::tests`
