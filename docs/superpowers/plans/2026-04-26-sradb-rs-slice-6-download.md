# sradb-rs Slice 6: Download Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land `sradb download <ACCESSION>...` — resolve accessions via metadata, fetch ENA fastq URLs, download HTTP files in parallel with `Range`-based resume + progress bars.

**Architecture:** A new `download.rs` module with `DownloadPlan` (list of (url, dest_path, expected_size, expected_md5)) and `download_plan()` that spawns N parallel HTTPS GETs with `Range: bytes=N-` for resume. Files write to `.part`, then atomically rename. Progress via `indicatif`'s `MultiProgress`. CLI takes accessions, resolves them via the slice-3 detailed metadata path (gives us ENA URLs), builds a plan, executes.

**Tech Stack:** `reqwest` streaming, `tokio::fs`, `indicatif`, `futures::stream::StreamExt`, existing slice-3 metadata orchestrator.

**Reference:** Slices 1-5 complete. FTP downloads, SRA prefetch tool, Aspera, and md5 verification are deferred.

---

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/download.rs` | `DownloadPlan`, `DownloadItem`, `download_plan()` (HTTP + resume + parallelism) |
| `crates/sradb-core/src/lib.rs` | (modify) `pub mod download;` |
| `crates/sradb-core/src/client.rs` | (modify) `SraClient::download` |
| `crates/sradb-cli/src/cmd/download.rs` | CLI handler (resolve accessions → plan → execute) |
| `crates/sradb-cli/src/cmd.rs` | (modify) `pub mod download;` |
| `crates/sradb-cli/src/main.rs` | (modify) register `Download` subcommand |
| `crates/sradb-cli/Cargo.toml` | (modify) add `indicatif.workspace = true` |
| `crates/sradb-core/tests/download_e2e.rs` | Wiremock test: small files + resume behavior |

---

## Task 1: DownloadPlan + DownloadItem types ✅

**Files:**
- Create: `crates/sradb-core/src/download.rs`
- Modify: `crates/sradb-core/src/lib.rs`

- [ ] **Step 1: Update lib.rs**

Read `lib.rs`. Insert `pub mod download;` (alphabetical — between `convert` and `ena`):

```rust
pub mod accession;
pub mod client;
pub mod convert;
pub mod download;
pub mod ena;
pub mod error;
pub mod http;
pub mod metadata;
pub mod model;
pub mod ncbi;
pub mod parse;
pub mod search;
```

(Search was added in slice 5; it stays.)

- [ ] **Step 2: Create download.rs (types only, no executor)**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/download.rs`:

```rust
//! Parallel HTTP downloads with `Range`-based resume.
//!
//! Slice 6 implements HTTP/HTTPS only. FTP, SRA prefetch, Aspera, and md5
//! verification are deferred.

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct DownloadItem {
    pub url: String,
    pub dest_path: PathBuf,
    /// Expected size in bytes. Used for progress reporting; `None` falls back to
    /// `Content-Length` from the HEAD response.
    pub expected_size: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DownloadPlan {
    pub items: Vec<DownloadItem>,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadReport {
    pub completed: u32,
    pub skipped: u32,
    pub failed: u32,
}
```

- [ ] **Step 3: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/download.rs
git commit -m "feat(download): DownloadPlan / DownloadItem / DownloadReport types"
```

---

## Task 2: Single-file HTTP download with Range resume ✅

**Files:**
- Modify: `crates/sradb-core/src/download.rs`

- [ ] **Step 1: Append download_one function**

Append to `download.rs`:

```rust

use crate::error::{Result, SradbError};
use futures::StreamExt;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Download one item to its dest path. Resumes if a `.part` file is present.
///
/// On success, the `.part` file is atomically renamed to `dest_path`.
/// Returns the bytes written (excluding any pre-existing partial bytes).
pub async fn download_one(http: &reqwest::Client, item: &DownloadItem) -> Result<u64> {
    if item.dest_path.exists() {
        // Already downloaded.
        return Ok(0);
    }
    if let Some(parent) = item.dest_path.parent() {
        fs::create_dir_all(parent).await.map_err(SradbError::Io)?;
    }
    let part_path = part_path(&item.dest_path);
    let resume_from = match fs::metadata(&part_path).await {
        Ok(m) => m.len(),
        Err(_) => 0,
    };

    let mut request = http.get(&item.url);
    if resume_from > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={resume_from}-"));
    }
    let resp = request.send().await.map_err(|source| SradbError::Http {
        endpoint: "download",
        source,
    })?;

    let status = resp.status();
    if !(status.is_success() || status == reqwest::StatusCode::PARTIAL_CONTENT) {
        return Err(SradbError::Download {
            url: item.url.clone(),
            reason: format!("unexpected status {status}"),
        });
    }

    let mut file = match (resume_from > 0, status == reqwest::StatusCode::PARTIAL_CONTENT) {
        (true, true) => fs::OpenOptions::new()
            .append(true)
            .open(&part_path)
            .await
            .map_err(SradbError::Io)?,
        // server didn't honor Range, restart from zero
        _ => fs::File::create(&part_path).await.map_err(SradbError::Io)?,
    };

    let mut written: u64 = 0;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|source| SradbError::Http {
            endpoint: "download",
            source,
        })?;
        file.write_all(&chunk).await.map_err(SradbError::Io)?;
        written += chunk.len() as u64;
    }
    file.flush().await.map_err(SradbError::Io)?;
    drop(file);

    fs::rename(&part_path, &item.dest_path)
        .await
        .map_err(SradbError::Io)?;
    Ok(written)
}

fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}
```

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/download.rs
git commit -m "feat(download): download_one with Range resume + atomic rename"
```

---

## Task 3: Parallel executor (download_plan) ✅

**Files:**
- Modify: `crates/sradb-core/src/download.rs`

- [ ] **Step 1: Append download_plan**

Append to `download.rs`:

```rust

use std::sync::Arc;
use tokio::sync::Semaphore;
use futures::stream::FuturesUnordered;

/// Execute a download plan with bounded parallelism.
pub async fn download_plan(
    http: &reqwest::Client,
    plan: &DownloadPlan,
    parallelism: usize,
) -> DownloadReport {
    let parallelism = parallelism.max(1);
    let semaphore = Arc::new(Semaphore::new(parallelism));
    let http = http.clone();

    let mut futures = FuturesUnordered::new();
    for item in &plan.items {
        let semaphore = semaphore.clone();
        let http = http.clone();
        let item = item.clone();
        futures.push(async move {
            let _permit = semaphore.acquire().await.expect("semaphore not closed");
            let res = download_one(&http, &item).await;
            (item, res)
        });
    }

    let mut report = DownloadReport::default();
    while let Some((item, res)) = futures.next().await {
        match res {
            Ok(0) if item.dest_path.exists() => {
                tracing::info!("skipping {} (already exists)", item.dest_path.display());
                report.skipped += 1;
            }
            Ok(_) => {
                tracing::info!("downloaded {}", item.dest_path.display());
                report.completed += 1;
            }
            Err(e) => {
                tracing::warn!("download failed for {}: {e}", item.url);
                report.failed += 1;
            }
        }
    }
    report
}
```

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/download.rs
git commit -m "feat(download): parallel executor with semaphore-bounded fan-out"
```

---

## Task 4: Wiremock e2e for download ✅

**Files:**
- Create: `crates/sradb-core/tests/download_e2e.rs`

- [ ] **Step 1: Write test**

Create `crates/sradb-core/tests/download_e2e.rs`:

```rust
//! End-to-end test of HTTP download with Range resume.

use sradb_core::download::{download_one, download_plan, DownloadItem, DownloadPlan};
use std::time::Duration;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn downloads_a_small_file() {
    let server = MockServer::start().await;
    let body = b"hello world".to_vec();
    Mock::given(method("GET"))
        .and(path("/foo.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("foo.txt");
    let item = DownloadItem {
        url: format!("{}/foo.txt", server.uri()),
        dest_path: dest.clone(),
        expected_size: Some(body.len() as u64),
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let written = download_one(&http, &item).await.unwrap();
    assert_eq!(written, body.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
}

#[tokio::test]
async fn skips_existing_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"x".to_vec()))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("foo.txt");
    std::fs::write(&dest, b"already here").unwrap();

    let item = DownloadItem {
        url: format!("{}/foo.txt", server.uri()),
        dest_path: dest.clone(),
        expected_size: None,
    };
    let http = reqwest::Client::builder().build().unwrap();
    let written = download_one(&http, &item).await.unwrap();
    assert_eq!(written, 0);
    assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
}

#[tokio::test]
async fn parallel_plan_executes_all() {
    let server = MockServer::start().await;
    for i in 0..5 {
        Mock::given(method("GET"))
            .and(path(format!("/{i}.txt")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 100]))
            .mount(&server)
            .await;
    }

    let tmp = TempDir::new().unwrap();
    let plan = DownloadPlan {
        items: (0..5)
            .map(|i| DownloadItem {
                url: format!("{}/{i}.txt", server.uri()),
                dest_path: tmp.path().join(format!("{i}.txt")),
                expected_size: Some(100),
            })
            .collect(),
    };
    let http = reqwest::Client::builder().build().unwrap();
    let report = download_plan(&http, &plan, 2).await;
    assert_eq!(report.completed, 5);
    assert_eq!(report.failed, 0);
    for i in 0..5 {
        let p = tmp.path().join(format!("{i}.txt"));
        assert!(p.exists());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 100);
    }
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sradb-core --test download_e2e 2>&1 | tail -10`
Expected: 3 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/download_e2e.rs
git commit -m "test(core): wiremock e2e for download (single, skip, parallel)"
```

---

## Task 5: SraClient::download method + CLI ✅

**Files:**
- Modify: `crates/sradb-core/src/client.rs`
- Create: `crates/sradb-cli/src/cmd/download.rs`
- Modify: `crates/sradb-cli/src/cmd.rs`
- Modify: `crates/sradb-cli/src/main.rs`
- Modify: `crates/sradb-cli/Cargo.toml`

- [ ] **Step 1: Append SraClient::download**

Inside `impl SraClient` (after `search` from slice 5), insert:

```rust

    /// Download a list of `DownloadItem`s with bounded parallelism.
    pub async fn download(
        &self,
        plan: &crate::download::DownloadPlan,
        parallelism: usize,
    ) -> crate::download::DownloadReport {
        // The `http` field is our reqwest-middleware wrapper; for raw streaming
        // downloads we use a fresh `reqwest::Client` with the same defaults.
        let raw = reqwest::Client::builder()
            .timeout(self.cfg.timeout)
            .user_agent(format!("sradb-rs/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client build");
        crate::download::download_plan(&raw, plan, parallelism).await
    }
```

- [ ] **Step 2: Update cli Cargo.toml**

Read `crates/sradb-cli/Cargo.toml`. Add `indicatif.workspace = true` to `[dependencies]`.

- [ ] **Step 3: Update cmd.rs**

```rust
//! Subcommand handlers.

pub mod convert;
pub mod download;
pub mod metadata;
pub mod search;
```

- [ ] **Step 4: Create cmd/download.rs**

Create `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/download.rs`:

```rust
//! `sradb download <ACCESSION>... [--out-dir ...] [-j N]` handler.

use std::path::PathBuf;

use clap::Args;
use sradb_core::download::{DownloadItem, DownloadPlan};
use sradb_core::{ClientConfig, MetadataOpts, SraClient};

#[derive(Args, Debug)]
pub struct DownloadArgs {
    /// One or more SRA accessions (SRP / SRX / SRR / GSE / GSM).
    #[arg(required = true)]
    pub accessions: Vec<String>,

    /// Output directory.
    #[arg(long, default_value = "./sradb_downloads")]
    pub out_dir: PathBuf,

    /// Parallel download workers.
    #[arg(short = 'j', long, default_value_t = 4)]
    pub parallelism: usize,
}

pub async fn run(args: DownloadArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let opts = MetadataOpts {
        detailed: true,
        enrich: false,
        page_size: 500,
    };

    let mut items: Vec<DownloadItem> = Vec::new();
    for acc in &args.accessions {
        let rows = client.metadata(acc, &opts).await?;
        for row in &rows {
            // Prefer ENA HTTPS URLs.
            for url in &row.run.urls.ena_fastq_http {
                let filename = url.rsplit('/').next().unwrap_or("download");
                let dest = args
                    .out_dir
                    .join(&row.run.study_accession)
                    .join(&row.run.experiment_accession)
                    .join(filename);
                items.push(DownloadItem {
                    url: url.clone(),
                    dest_path: dest,
                    expected_size: None,
                });
            }
        }
    }

    if items.is_empty() {
        eprintln!("no ENA fastq URLs found for the given accessions");
        std::process::exit(1);
    }

    let plan = DownloadPlan { items };
    println!("planning {} downloads (parallelism={})", plan.items.len(), args.parallelism);
    let report = client.download(&plan, args.parallelism).await;
    println!(
        "downloaded={}, skipped={}, failed={}",
        report.completed, report.skipped, report.failed
    );
    if report.failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}
```

- [ ] **Step 5: Update main.rs**

In the `Cmd` enum, add (after `Search`):

```rust
    /// Download SRA / ENA fastq files for accessions.
    Download(cmd::download::DownloadArgs),
```

In the match block:

```rust
        Some(Cmd::Download(args)) => cmd::download::run(args).await,
```

- [ ] **Step 6: Build**

Run: `cargo build --workspace 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 7: Smoke help**

Run: `cargo run -p sradb-cli --quiet -- download --help 2>&1 | tail -15`
Expected: clap help with `<ACCESSIONS>...`, `--out-dir`, `-j`/`--parallelism`.

- [ ] **Step 8: Commit**

```bash
git add crates/sradb-core/src/client.rs crates/sradb-cli/Cargo.toml crates/sradb-cli/src/cmd.rs crates/sradb-cli/src/cmd/download.rs crates/sradb-cli/src/main.rs
git commit -m "feat(cli): sradb download <accession>... -j N --out-dir DIR"
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
Expected: green; ≥76 tests.

- [ ] **Step 2: Mark + tag**

```bash
git tag -a slice-6-download -m "Slice 6: parallel HTTP downloads with Range resume"
```

---

## Deferred

- FTP downloads (suppaftp), Aspera, SRA prefetch tool integration
- md5/checksum verification
- `indicatif` MultiProgress UI (for now we log via tracing; CLI prints summary)
- Disk-space check before plan execution

## Definition of done

- `cargo test --workspace` green; ≥76 tests
- `sradb download SRP174132 --out-dir /tmp/sradb -j 2` against live ENA downloads at least one fastq file
- Wiremock e2e covers single-file, skip-existing, parallel
