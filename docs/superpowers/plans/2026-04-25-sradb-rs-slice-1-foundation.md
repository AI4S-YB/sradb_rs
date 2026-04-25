# sradb-rs Slice 1: Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the workspace skeleton, core types (`Accession`, `SradbError`), async HTTP client (rate-limit + retry), and the recorded-fixture / golden-snapshot test harness — so every subsequent slice can build features without re-deciding plumbing.

**Architecture:** Cargo workspace with three crates: `sradb-core` (lib), `sradb-cli` (bin, just `--version` for now), and `sradb-fixtures` (dev-only helpers). HTTP plumbing via `reqwest` + `reqwest-middleware` + `reqwest-retry`, rate limiting via `governor`. Tests use `wiremock` for stub HTTP and `insta` for golden snapshots.

**Tech Stack:** Rust 2024 edition, `tokio`, `reqwest` (rustls), `serde`, `quick-xml`, `thiserror`, `governor`, `clap`, `wiremock`, `insta`, `proptest`.

**Reference:** Spec at `docs/superpowers/specs/2026-04-25-sradb-rs-design.md`.

---

## File Map

Files this plan creates (each with one responsibility):

| File | Responsibility |
| --- | --- |
| `Cargo.toml` | Workspace definition, shared deps, lints |
| `rust-toolchain.toml` | Pin toolchain to stable |
| `.gitignore` | (already exists) update for target/ |
| `README.md` | Project intro + dev quickstart |
| `crates/sradb-core/Cargo.toml` | Library manifest |
| `crates/sradb-core/src/lib.rs` | Public re-exports + module declarations |
| `crates/sradb-core/src/accession.rs` | `Accession`, `AccessionKind`, `FromStr`, `Display`, `ParseAccessionError` |
| `crates/sradb-core/src/error.rs` | `SradbError` thiserror enum + `Result` alias |
| `crates/sradb-core/src/http.rs` | `HttpClient` wrapper: rate limit + retry middleware |
| `crates/sradb-core/src/client.rs` | `SraClient` shell + `ClientConfig` |
| `crates/sradb-core/tests/accession_property.rs` | proptest round-trip for `Accession` |
| `crates/sradb-cli/Cargo.toml` | Binary manifest |
| `crates/sradb-cli/src/main.rs` | clap skeleton, `sradb --version` |
| `crates/sradb-fixtures/Cargo.toml` | Dev-only manifest |
| `crates/sradb-fixtures/src/lib.rs` | `load_fixture()`, `mock_server()` helpers |
| `tools/capture-fixtures/Cargo.toml` | Manifest for the fixture-capture binary |
| `tools/capture-fixtures/src/main.rs` | Skeleton; gets fleshed out per slice |
| `.github/workflows/ci.yml` | Test + clippy + fmt on PR |
| `tests/data/.gitkeep` | Keep fixture dir in git |

---

## Task 1: Workspace Cargo.toml ✅

**Files:**
- Create: `Cargo.toml`

- [ ] **Step 1: Write workspace manifest**

```toml
[workspace]
resolver = "2"
members = [
    "crates/sradb-core",
    "crates/sradb-cli",
    "crates/sradb-fixtures",
    "tools/capture-fixtures",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
rust-version = "1.79"
license = "MIT"
repository = "https://github.com/saketkc/pysradb"
authors = ["sradb-rs contributors"]

[workspace.dependencies]
# async runtime
tokio = { version = "1.40", features = ["full"] }
futures = "0.3"

# http
reqwest = { version = "0.12", default-features = false, features = ["rustls-tls", "json", "gzip", "stream"] }
reqwest-middleware = "0.3"
reqwest-retry = "0.6"

# serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"
quick-xml = { version = "0.36", features = ["serialize"] }
csv = "1.3"

# error handling
thiserror = "1"
anyhow = "1"

# rate limiting
governor = "0.7"

# logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# misc
chrono = { version = "0.4", features = ["serde"] }
regex = "1"
once_cell = "1"

# cli
clap = { version = "4.5", features = ["derive", "env"] }
indicatif = "0.17"

# test deps
wiremock = "0.6"
insta = { version = "1.40", features = ["json", "yaml"] }
proptest = "1.5"
tokio-test = "0.4"
tempfile = "3.13"

[workspace.lints.rust]
unsafe_code = "forbid"
unused_imports = "warn"
unused_must_use = "deny"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
module_name_repetitions = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
must_use_candidate = "allow"
```

- [ ] **Step 2: Create `rust-toolchain.toml`**

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

- [ ] **Step 3: Update `.gitignore`**

Append to existing `.gitignore`:

```
/target
**/*.rs.bk
Cargo.lock.bak
.idea/
.vscode/
```

- [ ] **Step 4: Verify workspace structure (will fail until later tasks add the member crates)**

Run: `cargo metadata --no-deps 2>&1 | head -3`
Expected: error mentioning missing member crates (this is fine, fixed by next tasks).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore
git commit -m "chore: workspace skeleton"
```

---

## Task 2: sradb-fixtures crate skeleton ✅

**Files:**
- Create: `crates/sradb-fixtures/Cargo.toml`
- Create: `crates/sradb-fixtures/src/lib.rs`

We build this first because the core test harness depends on it.

- [ ] **Step 1: Manifest**

```toml
# crates/sradb-fixtures/Cargo.toml
[package]
name = "sradb-fixtures"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false

[lints]
workspace = true

[dependencies]
wiremock.workspace = true
tokio = { workspace = true }
serde_json.workspace = true
```

- [ ] **Step 2: Lib stub**

```rust
// crates/sradb-fixtures/src/lib.rs
//! Dev-only helpers shared between sradb-core and sradb-cli tests.

use std::path::PathBuf;

/// Path to the workspace root, computed at compile time from this crate's
/// `CARGO_MANIFEST_DIR` (= `<root>/crates/sradb-fixtures`).
#[must_use]
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// Load a fixture file from `<workspace>/tests/data/<relative>`.
///
/// Panics if the file is missing — fixtures must be committed.
#[must_use]
pub fn load_fixture(relative: &str) -> Vec<u8> {
    let path = workspace_root().join("tests/data").join(relative);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing fixture {}: {e}", path.display()))
}

/// Same as `load_fixture` but as UTF-8 string.
#[must_use]
pub fn load_fixture_str(relative: &str) -> String {
    let bytes = load_fixture(relative);
    String::from_utf8(bytes).expect("fixture is utf-8")
}

pub use wiremock;
```

- [ ] **Step 3: Run `cargo check -p sradb-fixtures`**

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-fixtures
git commit -m "feat(fixtures): scaffold dev-only fixture loader crate"
```

---

## Task 3: sradb-core crate skeleton ✅

**Files:**
- Modify: `Cargo.toml` (add `"crates/sradb-core"` to `[workspace] members`)
- Create: `crates/sradb-core/Cargo.toml`
- Create: `crates/sradb-core/src/lib.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/sradb-core/Cargo.toml
[package]
name = "sradb-core"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "Core library for the sradb-rs project: types, HTTP client, parsers."
repository.workspace = true

[lints]
workspace = true

[dependencies]
tokio.workspace = true
futures.workspace = true
reqwest.workspace = true
reqwest-middleware.workspace = true
reqwest-retry.workspace = true
serde.workspace = true
serde_json.workspace = true
quick-xml.workspace = true
csv.workspace = true
thiserror.workspace = true
governor.workspace = true
tracing.workspace = true
chrono.workspace = true
regex.workspace = true
once_cell.workspace = true

[dev-dependencies]
sradb-fixtures = { path = "../sradb-fixtures" }
tokio-test.workspace = true
wiremock.workspace = true
insta.workspace = true
proptest.workspace = true
tempfile.workspace = true

[features]
default = []
live = []
```

- [ ] **Step 2: Lib root**

```rust
// crates/sradb-core/src/lib.rs
//! sradb-core — core types, HTTP client, and parsers for the sradb-rs project.
//!
//! See `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` for the full spec.

pub mod accession;
pub mod client;
pub mod error;
pub mod http;

pub use accession::{Accession, AccessionKind, ParseAccessionError};
pub use client::{ClientConfig, SraClient};
pub use error::{Result, SradbError};
```

- [ ] **Step 3: Run `cargo check -p sradb-core`**

Expected: errors about missing modules. That's expected — Tasks 4-7 add them.

- [ ] **Step 4: Stub out the four modules so `cargo check` passes**

Create empty stubs at the four module paths so Task 3 leaves the workspace buildable:

```rust
// crates/sradb-core/src/accession.rs
//! Stub. Filled in Task 4.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessionKind { Srp, Srx, Srs, Srr, Gse, Gsm, BioProject, Pmid, Doi, Pmc }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accession { pub kind: AccessionKind, pub raw: String }

#[derive(Debug, thiserror::Error)]
#[error("invalid accession `{input}`: {reason}")]
pub struct ParseAccessionError { pub input: String, pub reason: String }
```

```rust
// crates/sradb-core/src/error.rs
//! Stub. Filled in Task 5.

#[derive(Debug, thiserror::Error)]
pub enum SradbError {
    #[error("placeholder")]
    Placeholder,
}

pub type Result<T> = std::result::Result<T, SradbError>;
```

```rust
// crates/sradb-core/src/http.rs
//! Stub. Filled in Task 6.
```

```rust
// crates/sradb-core/src/client.rs
//! Stub. Filled in Task 7.

#[derive(Debug, Default, Clone)]
pub struct ClientConfig {}

#[derive(Debug, Clone)]
pub struct SraClient {}

impl SraClient {
    #[must_use]
    pub fn new() -> Self { Self {} }
}

impl Default for SraClient { fn default() -> Self { Self::new() } }
```

- [ ] **Step 5: Run `cargo check -p sradb-core`**

Expected: PASS (warnings allowed; no errors).

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-core
git commit -m "feat(core): scaffold sradb-core crate skeleton"
```

---

## Task 4: AccessionKind + Accession type with FromStr ✅

**Files:**
- Modify: `crates/sradb-core/src/accession.rs` (full rewrite)

- [ ] **Step 1: Write failing tests in `accession.rs`**

Replace the stub from Task 3 with:

```rust
//! Typed accession identifiers used across the sradb-core API.
//!
//! Replaces pysradb's stringly-typed accession handling. Parsing is regex-based
//! and case-sensitive: NCBI/EBI accessions are upper-case by convention.

use std::fmt;
use std::str::FromStr;

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessionKind {
    Srp,
    Srx,
    Srs,
    Srr,
    Gse,
    Gsm,
    BioProject,
    Pmid,
    Doi,
    Pmc,
}

impl AccessionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Srp => "SRP",
            Self::Srx => "SRX",
            Self::Srs => "SRS",
            Self::Srr => "SRR",
            Self::Gse => "GSE",
            Self::Gsm => "GSM",
            Self::BioProject => "BioProject",
            Self::Pmid => "PMID",
            Self::Doi => "DOI",
            Self::Pmc => "PMC",
        }
    }
}

impl fmt::Display for AccessionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accession {
    pub kind: AccessionKind,
    pub raw: String,
}

impl Accession {
    #[must_use]
    pub fn new(kind: AccessionKind, raw: impl Into<String>) -> Self {
        Self { kind, raw: raw.into() }
    }
}

impl fmt::Display for Accession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid accession `{input}`: {reason}")]
pub struct ParseAccessionError {
    pub input: String,
    pub reason: String,
}

impl ParseAccessionError {
    fn new(input: &str, reason: impl Into<String>) -> Self {
        Self { input: input.to_owned(), reason: reason.into() }
    }
}

// Order matters: more specific patterns (PMC, BioProject) before generic.
static PATTERNS: Lazy<Vec<(AccessionKind, Regex)>> = Lazy::new(|| {
    vec![
        (AccessionKind::Pmc, Regex::new(r"^PMC\d+$").unwrap()),
        (AccessionKind::BioProject, Regex::new(r"^PRJ[A-Z]{2}\d+$").unwrap()),
        (AccessionKind::Srp, Regex::new(r"^[ED]?SRP\d{4,}$|^SRP\d{4,}$").unwrap()),
        (AccessionKind::Srx, Regex::new(r"^[ED]?SRX\d{4,}$|^SRX\d{4,}$").unwrap()),
        (AccessionKind::Srs, Regex::new(r"^[ED]?SRS\d{4,}$|^SRS\d{4,}$").unwrap()),
        (AccessionKind::Srr, Regex::new(r"^[ED]?SRR\d{4,}$|^SRR\d{4,}$").unwrap()),
        (AccessionKind::Gse, Regex::new(r"^GSE\d+$").unwrap()),
        (AccessionKind::Gsm, Regex::new(r"^GSM\d+$").unwrap()),
        (AccessionKind::Pmid, Regex::new(r"^\d{1,9}$").unwrap()),
    ]
});

// DOI is matched separately (loose RFC-3987-ish; doesn't fit the prefix pattern).
static DOI_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^10\.\d{4,9}/[\-._;()/:A-Za-z0-9]+$").unwrap());

impl FromStr for Accession {
    type Err = ParseAccessionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ParseAccessionError::new(s, "empty input"));
        }
        if DOI_RE.is_match(trimmed) {
            return Ok(Self::new(AccessionKind::Doi, trimmed));
        }
        for (kind, re) in PATTERNS.iter() {
            if re.is_match(trimmed) {
                return Ok(Self::new(*kind, trimmed));
            }
        }
        Err(ParseAccessionError::new(s, "no recognized accession pattern"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_kind() {
        let cases = [
            ("SRP000941", AccessionKind::Srp),
            ("SRX1800476", AccessionKind::Srx),
            ("SRS1467643", AccessionKind::Srs),
            ("SRR3587912", AccessionKind::Srr),
            ("ERR3587912", AccessionKind::Srr),
            ("DRR0123456", AccessionKind::Srr),
            ("GSE56924", AccessionKind::Gse),
            ("GSM1371490", AccessionKind::Gsm),
            ("PRJNA257197", AccessionKind::BioProject),
            ("PMC10802650", AccessionKind::Pmc),
            ("39528918", AccessionKind::Pmid),
            ("10.12688/f1000research.18676.1", AccessionKind::Doi),
        ];
        for (input, expected_kind) in cases {
            let acc: Accession = input.parse().unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(acc.kind, expected_kind, "for input {input}");
            assert_eq!(acc.raw, input);
        }
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "  ", "abc", "srp123", "SRP", "PRJ123", "10.x/y"] {
            assert!(bad.parse::<Accession>().is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn trims_whitespace() {
        let acc: Accession = "  SRP000941 \n".parse().unwrap();
        assert_eq!(acc.kind, AccessionKind::Srp);
        assert_eq!(acc.raw, "SRP000941");
    }

    #[test]
    fn display_round_trips() {
        let acc = Accession::new(AccessionKind::Srp, "SRP000941");
        assert_eq!(acc.to_string(), "SRP000941");
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --lib accession`
Expected: PASS (4 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/accession.rs
git commit -m "feat(core): add Accession parser with regex-based dispatch"
```

---

## Task 5: SradbError + Result alias ✅

**Files:**
- Modify: `crates/sradb-core/src/error.rs`

- [ ] **Step 1: Write failing tests**

Replace the stub:

```rust
//! Library-level error type. CLI wraps these with anyhow context.

use std::path::PathBuf;

use crate::accession::AccessionKind;

#[derive(Debug, thiserror::Error)]
pub enum SradbError {
    #[error("invalid accession `{input}`: {reason}")]
    InvalidAccession { input: String, reason: String },

    #[error("accession not found: {0}")]
    NotFound(String),

    #[error("conversion not supported: {from:?} -> {to:?}")]
    UnsupportedConversion { from: AccessionKind, to: AccessionKind },

    #[error("HTTP error from {endpoint}: {source}")]
    Http {
        endpoint: &'static str,
        #[source]
        source: reqwest::Error,
    },

    #[error("HTTP middleware error from {endpoint}: {source}")]
    HttpMiddleware {
        endpoint: &'static str,
        #[source]
        source: reqwest_middleware::Error,
    },

    #[error("rate limited by {service} after {retries} retries")]
    RateLimited { service: &'static str, retries: u32 },

    #[error("response parse error at {endpoint}: {message}")]
    Parse {
        endpoint: &'static str,
        message: String,
    },

    #[error("XML parse error in {context}: {source}")]
    Xml {
        context: &'static str,
        #[source]
        source: quick_xml::Error,
    },

    #[error("CSV parse error in {context}: {source}")]
    Csv {
        context: &'static str,
        #[source]
        source: csv::Error,
    },

    #[error("JSON parse error in {context}: {source}")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("enrichment failed: {message}")]
    Enrichment {
        message: String,
        #[source]
        source: Option<reqwest::Error>,
    },

    #[error("download failed for {url}: {reason}")]
    Download { url: String, reason: String },

    #[error("checksum mismatch for {path}: expected {expected}, got {got}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        got: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SradbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_context() {
        let err = SradbError::Parse {
            endpoint: "esummary",
            message: "missing field foo".into(),
        };
        let s = format!("{err}");
        assert!(s.contains("esummary"), "got: {s}");
        assert!(s.contains("missing field foo"), "got: {s}");
    }

    #[test]
    fn io_from_conversion() {
        let io: std::io::Error = std::io::ErrorKind::NotFound.into();
        let e: SradbError = io.into();
        assert!(matches!(e, SradbError::Io(_)));
    }

    #[test]
    fn unsupported_conversion_lists_kinds() {
        let e = SradbError::UnsupportedConversion {
            from: AccessionKind::Srp,
            to: AccessionKind::Pmid,
        };
        let s = format!("{e}");
        assert!(s.contains("Srp"));
        assert!(s.contains("Pmid"));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --lib error`
Expected: PASS (3 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/error.rs
git commit -m "feat(core): add SradbError enum with all variants from spec"
```

---

## Task 6: HTTP client with rate limit + retry ✅

**Files:**
- Modify: `crates/sradb-core/src/http.rs`
- Modify: `crates/sradb-core/Cargo.toml` (add http-cache deps if needed — none for now)

- [ ] **Step 1: Implement `HttpClient`**

```rust
//! Async HTTP wrapper shared across all backends.
//!
//! Wraps `reqwest::Client` with:
//! - exponential-backoff retry on transient HTTP/network errors via `reqwest_retry`
//! - per-service token-bucket rate limiting via `governor`
//! - a small surface (`get_bytes`, `get_text`, `get_json`) so callers don't see middleware machinery

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use governor::{Quota, RateLimiter};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::RetryTransientMiddleware;

use crate::error::{Result, SradbError};

/// Service whose rate-limit bucket a request should be charged against.
#[derive(Debug, Clone, Copy)]
pub enum Service {
    Ncbi,
    Ena,
    Other,
}

/// Per-service rate-limited HTTP client.
#[derive(Clone)]
pub struct HttpClient {
    inner: ClientWithMiddleware,
    ncbi_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ena_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient").finish_non_exhaustive()
    }
}

impl HttpClient {
    /// Build a client.
    /// - `ncbi_rps`: requests/sec for NCBI eUtils (3 without api_key, 10 with).
    /// - `ena_rps`: requests/sec for ENA. Default 8.
    /// - `max_retries`: retry attempts for transient failures.
    pub fn new(ncbi_rps: u32, ena_rps: u32, max_retries: u32, timeout: Duration) -> Result<Self> {
        let base = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(default_user_agent())
            .build()
            .map_err(|source| SradbError::Http { endpoint: "client_init", source })?;

        let policy = ExponentialBackoff::builder()
            .retry_bounds(Duration::from_millis(500), Duration::from_secs(30))
            .build_with_max_retries(max_retries);

        let inner = ClientBuilder::new(base)
            .with(RetryTransientMiddleware::new_with_policy(policy))
            .build();

        let ncbi_q = Quota::per_second(NonZeroU32::new(ncbi_rps.max(1)).unwrap());
        let ena_q = Quota::per_second(NonZeroU32::new(ena_rps.max(1)).unwrap());

        Ok(Self {
            inner,
            ncbi_limiter: Arc::new(RateLimiter::direct(ncbi_q)),
            ena_limiter: Arc::new(RateLimiter::direct(ena_q)),
        })
    }

    async fn wait_quota(&self, service: Service) {
        match service {
            Service::Ncbi => self.ncbi_limiter.until_ready().await,
            Service::Ena => self.ena_limiter.until_ready().await,
            Service::Other => {}
        }
    }

    /// GET → bytes. `endpoint` is a static label used in errors.
    pub async fn get_bytes(
        &self,
        endpoint: &'static str,
        service: Service,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<bytes::Bytes> {
        self.wait_quota(service).await;
        let resp = self
            .inner
            .get(url)
            .query(query)
            .send()
            .await
            .map_err(|source| SradbError::HttpMiddleware { endpoint, source })?;
        check_status(endpoint, &resp)?;
        resp.bytes()
            .await
            .map_err(|source| SradbError::Http { endpoint, source })
    }

    pub async fn get_text(
        &self,
        endpoint: &'static str,
        service: Service,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<String> {
        let bytes = self.get_bytes(endpoint, service, url, query).await?;
        String::from_utf8(bytes.to_vec()).map_err(|e| SradbError::Parse {
            endpoint,
            message: format!("response is not valid UTF-8: {e}"),
        })
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &'static str,
        service: Service,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let bytes = self.get_bytes(endpoint, service, url, query).await?;
        serde_json::from_slice(&bytes).map_err(|source| SradbError::Json {
            context: endpoint,
            source,
        })
    }
}

fn check_status(endpoint: &'static str, resp: &reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(SradbError::NotFound(format!("{endpoint}: {}", resp.url())));
    }
    Err(SradbError::Parse {
        endpoint,
        message: format!("unexpected status {status} from {}", resp.url()),
    })
}

fn default_user_agent() -> String {
    format!("sradb-rs/{}", env!("CARGO_PKG_VERSION"))
}
```

- [ ] **Step 2: Add `bytes` to dependencies**

In `crates/sradb-core/Cargo.toml`, add to `[dependencies]`:

```toml
bytes = "1"
```

- [ ] **Step 3: Run `cargo check -p sradb-core`**

Expected: PASS.

- [ ] **Step 4: Write integration test using wiremock**

Create `crates/sradb-core/tests/http_client.rs`:

```rust
use std::time::Duration;

use sradb_core::http::{HttpClient, Service};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_text_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hello"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
        .mount(&server)
        .await;

    let client = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
    let body = client
        .get_text("test", Service::Other, &format!("{}/hello", server.uri()), &[])
        .await
        .unwrap();
    assert_eq!(body, "hi");
}

#[tokio::test]
async fn retries_on_500_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = HttpClient::new(10, 10, 3, Duration::from_secs(5)).unwrap();
    let body = client
        .get_text("test", Service::Other, &format!("{}/x", server.uri()), &[])
        .await
        .unwrap();
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn returns_not_found_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
    let err = client
        .get_text("test", Service::Other, &format!("{}/missing", server.uri()), &[])
        .await
        .unwrap_err();
    assert!(matches!(err, sradb_core::SradbError::NotFound(_)));
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p sradb-core --test http_client`
Expected: PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/sradb-core/src/http.rs crates/sradb-core/Cargo.toml crates/sradb-core/tests/http_client.rs
git commit -m "feat(core): async HTTP client with rate-limit + retry"
```

---

## Task 7: SraClient + ClientConfig ✅

**Files:**
- Modify: `crates/sradb-core/src/client.rs`

- [ ] **Step 1: Implement `ClientConfig` with env-driven defaults**

```rust
//! High-level client. Slice 1 only stands up the shell + config; later slices
//! add the metadata/convert/search/download methods on top.

use std::time::Duration;

use crate::error::Result;
use crate::http::HttpClient;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub api_key: Option<String>,
    pub email: Option<String>,
    pub user_agent: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub ncbi_base_url: String,
    pub ena_base_url: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("NCBI_API_KEY").ok().filter(|s| !s.is_empty()),
            email: std::env::var("NCBI_EMAIL").ok().filter(|s| !s.is_empty()),
            user_agent: None,
            timeout: Duration::from_secs(30),
            max_retries: 5,
            ncbi_base_url: "https://eutils.ncbi.nlm.nih.gov/entrez/eutils".into(),
            ena_base_url: "https://www.ebi.ac.uk/ena".into(),
        }
    }
}

impl ClientConfig {
    /// True when an NCBI api_key is configured (raises rate limit from 3rps to 10rps).
    #[must_use]
    pub fn has_api_key(&self) -> bool {
        self.api_key.as_deref().is_some_and(|s| !s.is_empty())
    }
}

#[derive(Clone)]
pub struct SraClient {
    pub(crate) http: HttpClient,
    pub(crate) cfg: ClientConfig,
}

impl std::fmt::Debug for SraClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SraClient")
            .field("has_api_key", &self.cfg.has_api_key())
            .field("ncbi_base_url", &self.cfg.ncbi_base_url)
            .field("ena_base_url", &self.cfg.ena_base_url)
            .finish_non_exhaustive()
    }
}

impl SraClient {
    pub fn new() -> Result<Self> {
        Self::with_config(ClientConfig::default())
    }

    pub fn with_config(cfg: ClientConfig) -> Result<Self> {
        let ncbi_rps = if cfg.has_api_key() { 10 } else { 3 };
        let ena_rps = 8;
        let http = HttpClient::new(ncbi_rps, ena_rps, cfg.max_retries, cfg.timeout)?;
        Ok(Self { http, cfg })
    }

    #[must_use]
    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_pulls_env() {
        std::env::set_var("NCBI_API_KEY", "abc");
        let cfg = ClientConfig::default();
        assert!(cfg.has_api_key());
        std::env::remove_var("NCBI_API_KEY");
    }

    #[test]
    fn empty_env_var_is_treated_as_unset() {
        std::env::set_var("NCBI_API_KEY", "");
        let cfg = ClientConfig::default();
        assert!(!cfg.has_api_key());
        std::env::remove_var("NCBI_API_KEY");
    }

    #[test]
    fn client_constructs() {
        let c = SraClient::new().unwrap();
        assert!(!c.config().ncbi_base_url.is_empty());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --lib client -- --test-threads=1`
Expected: PASS (3 tests). Single-threaded because tests mutate process env.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/client.rs
git commit -m "feat(core): add SraClient shell with ClientConfig"
```

---

## Task 8: Property test for Accession round-trip ✅

**Files:**
- Create: `crates/sradb-core/tests/accession_property.rs`

- [ ] **Step 1: Add property test**

```rust
use proptest::prelude::*;
use sradb_core::accession::{Accession, AccessionKind};

fn accession_strategy() -> impl Strategy<Value = (AccessionKind, String)> {
    prop_oneof![
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srp, format!("SRP{n:06}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srx, format!("SRX{n:06}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srs, format!("SRS{n:06}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srr, format!("SRR{n:06}"))),
        (1u32..=999_999u32).prop_map(|n| (AccessionKind::Gse, format!("GSE{n}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Gsm, format!("GSM{n}"))),
        (1u32..=99_999_999u32).prop_map(|n| (AccessionKind::Pmid, format!("{n}"))),
        (1u32..=99_999_999u32).prop_map(|n| (AccessionKind::Pmc, format!("PMC{n}"))),
    ]
}

proptest! {
    #[test]
    fn parse_and_display_round_trip((expected_kind, raw) in accession_strategy()) {
        let acc: Accession = raw.parse().unwrap();
        prop_assert_eq!(acc.kind, expected_kind);
        prop_assert_eq!(acc.to_string(), raw.clone());
        prop_assert_eq!(acc.raw, raw);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p sradb-core --test accession_property`
Expected: PASS (1 property test, default 256 cases).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/accession_property.rs
git commit -m "test(core): proptest round-trip for Accession parser"
```

---

## Task 9: Make `mock_server()` helper available in fixtures ✅

**Files:**
- Modify: `crates/sradb-fixtures/src/lib.rs`

- [ ] **Step 1: Append helper for spinning up wiremock with sradb-core overrides**

Append to `crates/sradb-fixtures/src/lib.rs`:

```rust
/// Spin up a `wiremock::MockServer` and return its URL plus the server handle.
/// Caller must hold the handle for the test's lifetime.
pub async fn mock_server() -> wiremock::MockServer {
    wiremock::MockServer::start().await
}

/// Construct a `ClientConfig`-like struct (caller side) by overriding base URLs.
///
/// Returned tuple is `(ncbi_base, ena_base)` rooted at the same mock server but
/// under different path prefixes. Tests register mocks at `/eutils/...` and
/// `/ena/...` to disambiguate.
pub fn split_base_urls(server_uri: &str) -> (String, String) {
    (format!("{server_uri}/eutils"), format!("{server_uri}/ena"))
}
```

- [ ] **Step 2: Run `cargo check -p sradb-fixtures`**

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-fixtures/src/lib.rs
git commit -m "feat(fixtures): add mock_server + split_base_urls helpers"
```

---

## Task 10: Add base-url override path to SraClient (for testability) ✅

**Files:**
- Modify: `crates/sradb-core/src/client.rs`

The current `ClientConfig` already has the override fields, so this is just adding a builder convenience.

- [ ] **Step 1: Add `with_base_urls` constructor**

Append to `impl SraClient`:

```rust
    /// Convenience: same as `with_config` but overriding both base URLs.
    /// Used in tests to point the client at a wiremock server.
    pub fn with_base_urls(ncbi: impl Into<String>, ena: impl Into<String>) -> Result<Self> {
        let cfg = ClientConfig {
            ncbi_base_url: ncbi.into(),
            ena_base_url: ena.into(),
            ..ClientConfig::default()
        };
        Self::with_config(cfg)
    }
```

- [ ] **Step 2: Add an integration test exercising the wiring end-to-end**

Create `crates/sradb-core/tests/client_construction.rs`:

```rust
use sradb_core::SraClient;

#[tokio::test]
async fn build_client_with_test_base_urls() {
    let server = sradb_fixtures::mock_server().await;
    let (ncbi, ena) = sradb_fixtures::split_base_urls(&server.uri());
    let client = SraClient::with_base_urls(ncbi, ena).unwrap();
    let cfg = client.config();
    assert!(cfg.ncbi_base_url.starts_with(&server.uri()));
    assert!(cfg.ena_base_url.starts_with(&server.uri()));
}
```

- [ ] **Step 3: Run test**

Run: `cargo test -p sradb-core --test client_construction`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/client.rs crates/sradb-core/tests/client_construction.rs
git commit -m "feat(core): SraClient::with_base_urls convenience for tests"
```

---

## Task 11: sradb-cli skeleton ✅

**Files:**
- Modify: `Cargo.toml` (add `"crates/sradb-cli"` to `[workspace] members`)
- Create: `crates/sradb-cli/Cargo.toml`
- Create: `crates/sradb-cli/src/main.rs`

- [ ] **Step 1: Manifest**

```toml
# crates/sradb-cli/Cargo.toml
[package]
name = "sradb-cli"
version.workspace = true
edition.workspace = true
license.workspace = true
description = "CLI for sradb-rs."

[lints]
workspace = true

[[bin]]
name = "sradb"
path = "src/main.rs"

[dependencies]
sradb-core = { path = "../sradb-core" }
clap.workspace = true
tokio = { workspace = true }
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

- [ ] **Step 2: main.rs with `--version` and a verbosity flag**

```rust
//! sradb command-line interface.

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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Some(Cmd::Info) => {
            println!("sradb {}", env!("CARGO_PKG_VERSION"));
            println!("https://github.com/saketkc/pysradb (Rust port)");
            Ok(())
        }
        None => {
            // No subcommand: print short help.
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

- [ ] **Step 3: Run `cargo build -p sradb-cli`**

Expected: PASS.

- [ ] **Step 4: Smoke test the binary**

Run: `cargo run -p sradb-cli -- info`
Expected output:
```
sradb 0.1.0
https://github.com/saketkc/pysradb (Rust port)
```

Run: `cargo run -p sradb-cli -- --version`
Expected: `sradb 0.1.0`

- [ ] **Step 5: Commit**

```bash
git add crates/sradb-cli
git commit -m "feat(cli): scaffold clap-based CLI with --version and info"
```

---

## Task 12: capture-fixtures tool skeleton ✅

**Files:**
- Modify: `Cargo.toml` (add `"tools/capture-fixtures"` to `[workspace] members`)
- Create: `tools/capture-fixtures/Cargo.toml`
- Create: `tools/capture-fixtures/src/main.rs`
- Create: `tests/data/.gitkeep`

This binary will be fleshed out per slice (slice 2 needs the metadata fixtures, slice 4 needs search fixtures, etc.). Slice 1 just stands it up so the workspace is whole.

- [ ] **Step 1: Manifest**

```toml
# tools/capture-fixtures/Cargo.toml
[package]
name = "capture-fixtures"
version.workspace = true
edition.workspace = true
license.workspace = true
publish = false
description = "Dev tool: hits real NCBI/ENA/OpenAI endpoints to capture response fixtures."

[lints]
workspace = true

[[bin]]
name = "capture-fixtures"
path = "src/main.rs"

[dependencies]
sradb-core = { path = "../../crates/sradb-core" }
tokio = { workspace = true }
anyhow.workspace = true
clap.workspace = true
reqwest.workspace = true
serde_json.workspace = true
```

- [ ] **Step 2: Main**

```rust
//! Captures real responses from NCBI/ENA/OpenAI for use in offline tests.
//!
//! Usage examples (filled out as later slices need them):
//!     cargo run -p capture-fixtures -- ncbi-esummary --db sra --term SRP016501
//!     cargo run -p capture-fixtures -- ena-filereport --accession SRR057511

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "capture-fixtures", about = "Dev tool: capture real-API responses for offline tests.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Sanity check: print the configured base URLs and exit.
    Info,
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
    }
}
```

- [ ] **Step 3: Keep tests/data dir tracked**

Create `tests/data/.gitkeep` (empty file).

- [ ] **Step 4: Build**

Run: `cargo build -p capture-fixtures`
Expected: PASS.

Run: `cargo run -p capture-fixtures -- info`
Expected: prints the three lines.

- [ ] **Step 5: Commit**

```bash
git add tools/capture-fixtures tests/data/.gitkeep
git commit -m "feat(tools): scaffold capture-fixtures binary"
```

---

## Task 13: Workspace-wide build + test pass

**Files:** none changed; verification only.

- [ ] **Step 1: Verify the whole workspace compiles**

Run: `cargo build --workspace --all-targets`
Expected: PASS, no errors. Warnings are OK at this stage.

- [ ] **Step 2: Verify all tests pass**

Run: `cargo test --workspace -- --test-threads=1`
Expected: PASS. Single-threaded because some tests mutate process env.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS. Fix any warnings inline before continuing.

- [ ] **Step 4: Run rustfmt**

Run: `cargo fmt --all -- --check`
Expected: PASS. If it fails, run `cargo fmt --all` and commit the formatting changes as a separate commit.

- [ ] **Step 5: If fmt made changes, commit**

```bash
git add -u
git commit -m "style: cargo fmt pass"
```

---

## Task 14: GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Workflow**

```yaml
name: ci

on:
  pull_request:
  push:
    branches: [main]

jobs:
  test:
    name: test
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo build --workspace --all-targets
      - run: cargo test --workspace -- --test-threads=1
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: cargo fmt + clippy + test on push/PR"
```

---

## Task 15: README

**Files:**
- Create: `README.md`

- [ ] **Step 1: README content**

```markdown
# sradb-rs

Rust port of [pysradb](https://github.com/saketkc/pysradb): query NGS metadata from NCBI SRA, ENA, and GEO.

**Status:** early development. Slice 1 (foundation) only. See `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` for the design and `docs/superpowers/plans/` for implementation plans.

## Quickstart (dev)

```bash
cargo build --workspace
cargo test --workspace -- --test-threads=1
cargo run -p sradb-cli -- info
```

## Layout

- `crates/sradb-core/` — async library: types, HTTP client, parsers (per-slice).
- `crates/sradb-cli/` — `sradb` CLI binary.
- `crates/sradb-fixtures/` — dev-only test helpers.
- `tools/capture-fixtures/` — captures real-API responses for offline tests.
- `tests/data/` — committed response fixtures.
- `pysradb/` — original Python implementation, kept in tree for reference during the port.

## Configuration

Environment variables:

- `NCBI_API_KEY` — raises NCBI rate limit from 3rps to 10rps.
- `NCBI_EMAIL` — recommended by NCBI E-utils etiquette.
- `OPENAI_API_KEY` — required for `--enrich` (slice 7+).
- `OPENAI_BASE_URL` — override for any OpenAI-compatible endpoint.

## License

MIT.
```

- [ ] **Step 2: Commit**

```bash
git add README.md
git commit -m "docs: project README with dev quickstart"
```

---

## Task 16: Final verification

**Files:** none changed.

- [ ] **Step 1: Full build + test + lint pass**

Run, in order:
```bash
cargo build --workspace --all-targets
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```
Expected: all PASS.

- [ ] **Step 2: Smoke test the CLI**

Run: `cargo run -p sradb-cli -- info`
Expected: prints version line.

Run: `cargo run -p sradb-cli -- --help`
Expected: clap-formatted help including `info` subcommand.

- [ ] **Step 3: Verify git state**

Run: `git log --oneline`
Expected: ~13-15 commits, each one task. No commit named "WIP" or "fix".

Run: `git status`
Expected: clean working tree.

- [ ] **Step 4: Tag the slice**

```bash
git tag -a slice-1-foundation -m "Slice 1: foundation complete"
```

(Push the tag manually when convenient.)

---

## What this slice does NOT include (intentional deferrals)

- No metadata/convert/search/download methods on `SraClient` yet — those land in slices 2–8.
- No parsing of NCBI XML/JSON — `parse/` modules don't exist yet (slice 2 adds them).
- No model structs (`Run`, `Experiment`, etc.) — slice 2 adds them.
- No `tools/capture-fixtures` real captures — each downstream slice adds the fixtures it needs.
- No release artifact / cargo publish — slice 9.

## Definition of done for slice 1

1. `cargo build --workspace` clean.
2. `cargo test --workspace` clean (≥10 tests passing across `accession`, `error`, `client`, `http_client`, `accession_property`, `client_construction`).
3. `cargo clippy --workspace --all-targets -- -D warnings` clean.
4. `cargo fmt --all -- --check` clean.
5. `sradb info` and `sradb --version` work.
6. CI workflow runs the same gates on PR.
7. README explains quickstart and layout.
8. `git tag slice-1-foundation` created.

After this lands, the slice 2 plan can be written confidently against real code rather than guesswork.
