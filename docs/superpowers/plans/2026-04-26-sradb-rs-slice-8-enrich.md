# sradb-rs Slice 8: LLM Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Land `sradb metadata <ACC> --enrich` — for each row, send the sample text to an OpenAI-compatible chat completions endpoint with a structured-output JSON schema, populate `MetadataRow.enrichment` (9 ontology fields).

**Architecture:** A new `enrich.rs` module with `EnrichConfig`, prompt builder, response parser, and a semaphore-bounded executor. Hooks into the metadata orchestrator after the slice-3 detailed augmentation when `opts.enrich = true`. Reuses existing `model::Enrichment` struct (defined in slice 2).

**Tech Stack:** `reqwest` POST with JSON body, `serde_json` for the OpenAI request/response shape, `tokio::sync::Semaphore` + `FuturesUnordered` for fan-out, env vars `OPENAI_API_KEY` and optional `OPENAI_BASE_URL`.

**Reference:** Spec at `docs/superpowers/specs/2026-04-25-sradb-rs-design.md` (sections "Enrichment" and "Error handling"). Slices 1-7 complete.

---

## Background: OpenAI chat completions wire format

```
POST {base_url}/v1/chat/completions
Authorization: Bearer {api_key}
Content-Type: application/json
```

Body:
```json
{
  "model": "gpt-4o-mini",
  "temperature": 0.0,
  "messages": [
    {"role": "system", "content": "Extract biological metadata fields from the provided sample text. Return null for fields not determinable."},
    {"role": "user", "content": "<concatenated sample/experiment titles + sample attributes>"}
  ],
  "response_format": {
    "type": "json_schema",
    "json_schema": {
      "name": "metadata_extraction",
      "strict": true,
      "schema": {
        "type": "object",
        "additionalProperties": false,
        "required": ["organ", "tissue", "anatomical_system", "cell_type", "disease", "sex", "development_stage", "assay", "organism"],
        "properties": {
          "organ":              {"type": ["string", "null"]},
          "tissue":             {"type": ["string", "null"]},
          "anatomical_system":  {"type": ["string", "null"]},
          "cell_type":          {"type": ["string", "null"]},
          "disease":            {"type": ["string", "null"]},
          "sex":                {"type": ["string", "null"]},
          "development_stage":  {"type": ["string", "null"]},
          "assay":              {"type": ["string", "null"]},
          "organism":           {"type": ["string", "null"]}
        }
      }
    }
  }
}
```

Response:
```json
{
  "id": "...",
  "choices": [{
    "message": {"role": "assistant", "content": "{\"organ\": \"liver\", \"tissue\": null, ...}"}
  }]
}
```

The `content` field is a JSON-encoded string matching our schema. We parse it into the existing `Enrichment` struct.

## File Map

| File | Responsibility |
| --- | --- |
| `crates/sradb-core/src/enrich.rs` | `EnrichConfig`, prompt builder, OpenAI client, response parser, executor |
| `crates/sradb-core/src/lib.rs` | (modify) `pub mod enrich;` |
| `crates/sradb-core/src/error.rs` | already has `Enrichment { message, source }` variant from slice 1 — no changes |
| `crates/sradb-core/src/client.rs` | (modify) `SraClient::enrich_rows` method |
| `crates/sradb-core/src/metadata.rs` | (modify) call `enrich_rows` when `opts.enrich = true` |
| `crates/sradb-cli/src/cmd/metadata.rs` | (modify) add `--enrich` flag |
| `crates/sradb-core/tests/enrich_e2e.rs` | Wiremock e2e covering happy path + missing API key |

---

## Task 1: EnrichConfig + prompt builder + types

**Files:**
- Modify: `crates/sradb-core/src/lib.rs`
- Create: `crates/sradb-core/src/enrich.rs`

- [ ] **Step 1: Update lib.rs**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/lib.rs`. Currently has `convert`, `download`, `ena`, `error`, `geo` etc. Add `pub mod enrich;` (alphabetical between `ena` and `error`):

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
pub mod metadata;
pub mod model;
pub mod ncbi;
pub mod parse;
pub mod search;
```

- [ ] **Step 2: Create enrich.rs (config + prompt + parsing only)**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/src/enrich.rs`:

```rust
//! LLM-based metadata enrichment via an OpenAI-compatible chat completions API.
//!
//! For each `MetadataRow`, construct a prompt from the sample/experiment titles
//! and SAMPLE_ATTRIBUTES bag, send to the configured chat-completions endpoint
//! with a strict JSON schema response format, and decode the result into the
//! 9-field `Enrichment` struct.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Result, SradbError};
use crate::model::{Enrichment, MetadataRow};

const DEFAULT_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_MODEL: &str = "gpt-4o-mini";
const DEFAULT_TIMEOUT_SECS: u64 = 60;
const DEFAULT_CONCURRENCY: usize = 8;
const DEFAULT_MAX_RETRIES: u32 = 3;

const SYSTEM_PROMPT: &str = "Extract biological metadata fields from the provided sample text. \
Return null for fields not determinable.";

/// Enrichment configuration.
#[derive(Debug, Clone)]
pub struct EnrichConfig {
    /// `OPENAI_API_KEY` (required when calling `enrich`).
    pub api_key: String,
    /// `OPENAI_BASE_URL` override (default `https://api.openai.com`).
    pub base_url: String,
    pub model: String,
    pub temperature: f32,
    pub concurrency: usize,
    pub max_retries: u32,
    pub timeout: Duration,
}

impl EnrichConfig {
    /// Build a config from environment variables.
    /// Returns `None` if `OPENAI_API_KEY` is unset or empty.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").ok().filter(|s| !s.is_empty())?;
        let base_url = std::env::var("OPENAI_BASE_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        let model = std::env::var("OPENAI_MODEL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        Some(Self {
            api_key,
            base_url,
            model,
            temperature: 0.0,
            concurrency: DEFAULT_CONCURRENCY,
            max_retries: DEFAULT_MAX_RETRIES,
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        })
    }
}

/// Build the user-message prompt for one `MetadataRow`. Concatenates sample
/// title, experiment title, and the sample attribute bag.
#[must_use]
pub fn build_prompt(row: &MetadataRow) -> String {
    let mut out = String::new();
    if let Some(t) = &row.experiment.title {
        out.push_str("experiment_title: ");
        out.push_str(t);
        out.push('\n');
    }
    if let Some(t) = &row.sample.title {
        out.push_str("sample_title: ");
        out.push_str(t);
        out.push('\n');
    }
    if let Some(o) = &row.sample.organism_name {
        out.push_str("organism_name: ");
        out.push_str(o);
        out.push('\n');
    }
    if !row.sample.attributes.is_empty() {
        out.push_str("sample_attributes:\n");
        for (k, v) in &row.sample.attributes {
            out.push_str("  ");
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push('\n');
        }
    }
    out.trim_end().to_owned()
}

// --- OpenAI request / response shapes ---

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    temperature: f32,
    messages: Vec<ChatMessage<'a>>,
    response_format: ResponseFormat,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
    json_schema: JsonSchemaSpec,
}

#[derive(Debug, Serialize)]
struct JsonSchemaSpec {
    name: &'static str,
    strict: bool,
    schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: String,
}

/// JSON schema for the structured-output response.
fn enrichment_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "organ", "tissue", "anatomical_system", "cell_type", "disease",
            "sex", "development_stage", "assay", "organism"
        ],
        "properties": {
            "organ":             {"type": ["string", "null"]},
            "tissue":            {"type": ["string", "null"]},
            "anatomical_system": {"type": ["string", "null"]},
            "cell_type":         {"type": ["string", "null"]},
            "disease":           {"type": ["string", "null"]},
            "sex":               {"type": ["string", "null"]},
            "development_stage": {"type": ["string", "null"]},
            "assay":             {"type": ["string", "null"]},
            "organism":          {"type": ["string", "null"]}
        }
    })
}

/// Build the OpenAI request body for a single user prompt.
#[must_use]
pub fn build_request_body(model: &str, temperature: f32, user_message: &str) -> serde_json::Value {
    let req = ChatRequest {
        model,
        temperature,
        messages: vec![
            ChatMessage { role: "system", content: SYSTEM_PROMPT },
            ChatMessage { role: "user", content: user_message },
        ],
        response_format: ResponseFormat {
            kind: "json_schema",
            json_schema: JsonSchemaSpec {
                name: "metadata_extraction",
                strict: true,
                schema: enrichment_schema(),
            },
        },
    };
    serde_json::to_value(req).expect("serialize ChatRequest")
}

/// Parse the OpenAI response body into an `Enrichment`. The response's
/// `choices[0].message.content` is a JSON-encoded string matching our schema.
pub fn parse_response(body: &str) -> Result<Enrichment> {
    let resp: ChatResponse = serde_json::from_str(body).map_err(|source| SradbError::Json {
        context: "openai_chat",
        source,
    })?;
    let content = resp.choices.first().map(|c| &c.message.content).ok_or_else(|| SradbError::Parse {
        endpoint: "openai_chat",
        message: "response has no choices".into(),
    })?;
    let enrichment: Enrichment = serde_json::from_str(content).map_err(|source| SradbError::Json {
        context: "openai_chat_content",
        source,
    })?;
    Ok(enrichment)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Experiment, Library, Platform, Run, RunUrls, Sample, Study};
    use std::collections::BTreeMap;

    fn fixture_row() -> MetadataRow {
        let mut attrs = BTreeMap::new();
        attrs.insert("source_name".into(), "liver".into());
        attrs.insert("cell type".into(), "hepatocyte".into());
        MetadataRow {
            run: Run::default(),
            experiment: Experiment {
                title: Some("RNA-Seq of liver hepatocytes".into()),
                library: Library::default(),
                platform: Platform::default(),
                ..Experiment::default()
            },
            sample: Sample {
                title: Some("Liver sample 1".into()),
                organism_name: Some("Homo sapiens".into()),
                attributes: attrs,
                ..Sample::default()
            },
            study: Study::default(),
            enrichment: None,
        }
    }

    #[test]
    fn build_prompt_concatenates_fields() {
        let row = fixture_row();
        let p = build_prompt(&row);
        assert!(p.contains("experiment_title: RNA-Seq of liver hepatocytes"));
        assert!(p.contains("sample_title: Liver sample 1"));
        assert!(p.contains("organism_name: Homo sapiens"));
        assert!(p.contains("source_name: liver"));
        assert!(p.contains("cell type: hepatocyte"));
    }

    #[test]
    fn build_prompt_handles_empty_row() {
        let row = MetadataRow {
            run: Run::default(),
            experiment: Experiment::default(),
            sample: Sample::default(),
            study: Study::default(),
            enrichment: None,
        };
        let prompt = build_prompt(&row);
        assert_eq!(prompt, "", "empty row should produce empty prompt");
    }

    #[test]
    fn build_request_body_has_required_shape() {
        let v = build_request_body("gpt-4o-mini", 0.0, "hello");
        assert_eq!(v["model"], "gpt-4o-mini");
        assert_eq!(v["messages"][0]["role"], "system");
        assert_eq!(v["messages"][1]["role"], "user");
        assert_eq!(v["messages"][1]["content"], "hello");
        assert_eq!(v["response_format"]["type"], "json_schema");
        assert_eq!(v["response_format"]["json_schema"]["name"], "metadata_extraction");
        assert_eq!(v["response_format"]["json_schema"]["strict"], true);
        let props = &v["response_format"]["json_schema"]["schema"]["properties"];
        for field in ["organ", "tissue", "anatomical_system", "cell_type", "disease", "sex", "development_stage", "assay", "organism"] {
            assert!(props[field].is_object(), "missing field {field}");
        }
    }

    #[test]
    fn parse_response_typical() {
        let body = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"{\"organ\":\"liver\",\"tissue\":null,\"anatomical_system\":\"hepatobiliary system\",\"cell_type\":\"hepatocyte\",\"disease\":null,\"sex\":null,\"development_stage\":null,\"assay\":\"RNA-Seq\",\"organism\":\"Homo sapiens\"}"}}]}"#;
        let e = parse_response(body).unwrap();
        assert_eq!(e.organ.as_deref(), Some("liver"));
        assert_eq!(e.cell_type.as_deref(), Some("hepatocyte"));
        assert_eq!(e.assay.as_deref(), Some("RNA-Seq"));
        assert_eq!(e.tissue, None);
    }

    #[test]
    fn parse_response_no_choices_errors() {
        let body = r#"{"choices":[]}"#;
        assert!(parse_response(body).is_err());
    }
}
```

- [ ] **Step 3: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test -p sradb-core --lib enrich 2>&1 | tail -5`
Expected: 5 tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-core/src/lib.rs crates/sradb-core/src/enrich.rs
git commit -m "feat(enrich): EnrichConfig + prompt builder + OpenAI request/response shape + parsers"
```

---

## Task 2: Single-row enrichment executor

**Files:**
- Modify: `crates/sradb-core/src/enrich.rs`

- [ ] **Step 1: Append `enrich_one` function**

Append (do not replace) to `crates/sradb-core/src/enrich.rs`:

```rust

/// Send one prompt to the chat-completions endpoint and parse the response.
pub async fn enrich_one(
    http: &reqwest::Client,
    cfg: &EnrichConfig,
    user_message: &str,
) -> Result<Enrichment> {
    let url = format!("{}/v1/chat/completions", cfg.base_url);
    let body = build_request_body(&cfg.model, cfg.temperature, user_message);

    let resp = http
        .post(&url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .await
        .map_err(|source| SradbError::Enrichment {
            message: format!("HTTP error to {url}"),
            source: Some(source),
        })?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(SradbError::Enrichment {
            message: format!("status {status} from {url}: {}", truncate(&text, 256)),
            source: None,
        });
    }
    let text = resp.text().await.map_err(|source| SradbError::Enrichment {
        message: "reading body".into(),
        source: Some(source),
    })?;
    parse_response(&text)
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_owned()
    } else {
        let mut out = s[..n].to_owned();
        out.push_str("...");
        out
    }
}
```

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test -p sradb-core --lib enrich 2>&1 | tail -5`
Expected: 5 tests PASS (existing — no new tests, since `enrich_one` is exercised via integration test in Task 4).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/enrich.rs
git commit -m "feat(enrich): enrich_one executor for single-row LLM call"
```

---

## Task 3: Parallel `enrich_rows` executor

**Files:**
- Modify: `crates/sradb-core/src/enrich.rs`

- [ ] **Step 1: Append `enrich_rows` function**

Append:

```rust

use std::sync::Arc;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use tokio::sync::Semaphore;

/// Enrich each row in `rows` in parallel (semaphore-bounded). Per-row failures
/// are logged via `tracing::warn!` and leave `row.enrichment = None` —
/// individual failures never abort the batch.
pub async fn enrich_rows(cfg: &EnrichConfig, rows: &mut [MetadataRow]) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let http = reqwest::Client::builder()
        .timeout(cfg.timeout)
        .build()
        .map_err(|source| SradbError::Enrichment {
            message: "build reqwest client".into(),
            source: Some(source),
        })?;

    let semaphore = Arc::new(Semaphore::new(cfg.concurrency.max(1)));
    let mut futures = FuturesUnordered::new();
    for (idx, row) in rows.iter().enumerate() {
        let prompt = build_prompt(row);
        let semaphore = semaphore.clone();
        let http = http.clone();
        let cfg = cfg.clone();
        futures.push(async move {
            let _permit = semaphore.acquire().await.expect("semaphore not closed");
            let res = enrich_one(&http, &cfg, &prompt).await;
            (idx, res)
        });
    }

    while let Some((idx, res)) = futures.next().await {
        match res {
            Ok(e) => {
                rows[idx].enrichment = Some(e);
            }
            Err(err) => {
                tracing::warn!("enrichment failed for row {idx}: {err}");
            }
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test -p sradb-core --lib enrich 2>&1 | tail -5`
Expected: 5 tests PASS (no new tests; integration covered in Task 4).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/enrich.rs
git commit -m "feat(enrich): enrich_rows parallel executor with semaphore + per-row failure isolation"
```

---

## Task 4: Wiremock e2e

**Files:**
- Create: `crates/sradb-core/tests/enrich_e2e.rs`

- [ ] **Step 1: Write tests**

Create `/home/xzg/project/sradb_rs/crates/sradb-core/tests/enrich_e2e.rs`:

```rust
//! End-to-end test of the enrichment flow against a wiremock OpenAI endpoint.

use std::collections::BTreeMap;
use std::time::Duration;

use sradb_core::enrich::{enrich_rows, EnrichConfig};
use sradb_core::model::{Experiment, Library, MetadataRow, Platform, Run, Sample, Study};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_row() -> MetadataRow {
    let mut attrs = BTreeMap::new();
    attrs.insert("source_name".into(), "liver".into());
    MetadataRow {
        run: Run::default(),
        experiment: Experiment {
            title: Some("RNA-Seq of liver".into()),
            library: Library::default(),
            platform: Platform::default(),
            ..Experiment::default()
        },
        sample: Sample {
            title: Some("Liver sample 1".into()),
            organism_name: Some("Homo sapiens".into()),
            attributes: attrs,
            ..Sample::default()
        },
        study: Study::default(),
        enrichment: None,
    }
}

#[tokio::test]
async fn enrich_rows_populates_enrichment_field() {
    let server = MockServer::start().await;
    let body = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"{\"organ\":\"liver\",\"tissue\":null,\"anatomical_system\":\"hepatobiliary system\",\"cell_type\":\"hepatocyte\",\"disease\":null,\"sex\":null,\"development_stage\":null,\"assay\":\"RNA-Seq\",\"organism\":\"Homo sapiens\"}"}}]}"#;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let cfg = EnrichConfig {
        api_key: "sk-test".into(),
        base_url: server.uri(),
        model: "gpt-4o-mini".into(),
        temperature: 0.0,
        concurrency: 4,
        max_retries: 0,
        timeout: Duration::from_secs(5),
    };
    let mut rows = vec![fixture_row()];
    enrich_rows(&cfg, &mut rows).await.unwrap();
    let e = rows[0].enrichment.as_ref().expect("enrichment should be Some");
    assert_eq!(e.organ.as_deref(), Some("liver"));
    assert_eq!(e.cell_type.as_deref(), Some("hepatocyte"));
    assert_eq!(e.assay.as_deref(), Some("RNA-Seq"));
}

#[tokio::test]
async fn enrich_rows_per_row_failure_isolated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let cfg = EnrichConfig {
        api_key: "sk-test".into(),
        base_url: server.uri(),
        model: "gpt-4o-mini".into(),
        temperature: 0.0,
        concurrency: 4,
        max_retries: 0,
        timeout: Duration::from_secs(5),
    };
    let mut rows = vec![fixture_row(), fixture_row()];
    enrich_rows(&cfg, &mut rows).await.unwrap();
    // Both should fail without panicking; enrichment stays None.
    assert!(rows[0].enrichment.is_none());
    assert!(rows[1].enrichment.is_none());
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sradb-core --test enrich_e2e 2>&1 | tail -10`
Expected: 2 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/tests/enrich_e2e.rs
git commit -m "test(enrich): wiremock e2e for happy path + per-row failure isolation"
```

---

## Task 5: Hook into metadata orchestrator

**Files:**
- Modify: `crates/sradb-core/src/metadata.rs`

- [ ] **Step 1: Append enrichment branch in fetch_metadata**

Read `/home/xzg/project/sradb_rs/crates/sradb-core/src/metadata.rs`. Find the end of the `if !opts.detailed { return Ok(rows); }` block. Detailed augmentation runs after — we add enrichment after that.

Locate this block (the last thing in `fetch_metadata`):

```rust
    if !opts.detailed {
        return Ok(rows);
    }

    // Detailed-mode augmentation.
    augment_with_runinfo(...).await?;
    augment_with_experiment_package(...).await?;
    augment_with_ena_fastq(http, ena_base_url, &mut rows).await?;
    Ok(rows)
}
```

Replace the trailing `Ok(rows)` with the enrichment branch:

```rust
    if !opts.detailed {
        return Ok(rows);
    }

    // Detailed-mode augmentation.
    augment_with_runinfo(...).await?;
    augment_with_experiment_package(...).await?;
    augment_with_ena_fastq(http, ena_base_url, &mut rows).await?;

    if opts.enrich {
        if let Some(cfg) = crate::enrich::EnrichConfig::from_env() {
            crate::enrich::enrich_rows(&cfg, &mut rows).await?;
        } else {
            return Err(crate::error::SradbError::Enrichment {
                message: "OPENAI_API_KEY not set; cannot enrich".into(),
                source: None,
            });
        }
    }

    Ok(rows)
}
```

(Keep the existing `augment_with_*` calls unchanged — they're shown abbreviated above. Insert the `if opts.enrich { ... }` block between the last augment call and `Ok(rows)`.)

- [ ] **Step 2: Build + tests**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

Run: `cargo test --workspace 2>&1 | tail -3`
Expected: PASS, total ≥ 90 (slice 7 baseline 83 + 5 enrich unit + 2 enrich e2e).

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/metadata.rs
git commit -m "feat(metadata): wire enrich_rows into fetch_metadata when opts.enrich + OPENAI_API_KEY set"
```

---

## Task 6: CLI --enrich flag

**Files:**
- Modify: `crates/sradb-cli/src/cmd/metadata.rs`

- [ ] **Step 1: Add --enrich flag**

Read `/home/xzg/project/sradb_rs/crates/sradb-cli/src/cmd/metadata.rs`. Find the `MetadataArgs` struct. Insert a new field after `detailed`:

```rust
    /// Enrich each row with LLM-extracted ontology fields (organ, tissue, etc.).
    /// Requires OPENAI_API_KEY env var. Optionally OPENAI_BASE_URL and OPENAI_MODEL.
    #[arg(long, default_value_t = false)]
    pub enrich: bool,
```

- [ ] **Step 2: Plumb into MetadataOpts**

Find the `MetadataOpts { ... }` literal in the `run` function. Currently:

```rust
    let opts = MetadataOpts {
        detailed: args.detailed,
        enrich: false,
        page_size: args.page_size,
    };
```

Change `enrich: false` to `enrich: args.enrich`:

```rust
    let opts = MetadataOpts {
        detailed: args.detailed,
        enrich: args.enrich,
        page_size: args.page_size,
    };
```

- [ ] **Step 3: Build + smoke help**

Run: `cargo build -p sradb-cli 2>&1 | tail -3`
Expected: PASS.

Run: `cargo run -p sradb-cli --quiet -- metadata --help 2>&1 | tail -15`
Expected: clap help including `--enrich`.

- [ ] **Step 4: Commit**

```bash
git add crates/sradb-cli/src/cmd/metadata.rs
git commit -m "feat(cli): --enrich flag on sradb metadata"
```

## Task 7: SraClient::enrich_rows facade (optional convenience)

**Files:**
- Modify: `crates/sradb-core/src/client.rs`

- [ ] **Step 1: Append SraClient::enrich_rows**

Inside `impl SraClient`, after `geo_matrix_download`:

```rust

    /// Enrich a list of metadata rows in place using LLM-extracted fields.
    /// `EnrichConfig` is typically built via `EnrichConfig::from_env()`.
    pub async fn enrich_rows(
        &self,
        cfg: &crate::enrich::EnrichConfig,
        rows: &mut [crate::model::MetadataRow],
    ) -> Result<()> {
        crate::enrich::enrich_rows(cfg, rows).await
    }
```

- [ ] **Step 2: Build**

Run: `cargo build -p sradb-core 2>&1 | tail -3`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sradb-core/src/client.rs
git commit -m "feat(client): SraClient::enrich_rows facade"
```

---

## Task 8: Final verification

- [ ] **Step 1: All gates**

```bash
cargo build --workspace --all-targets 2>&1 | tail -3
cargo fmt --all -- --check 2>&1 | tail -2
RUSTFLAGS="-Dwarnings" cargo clippy --workspace --all-targets 2>&1 | tail -3
cargo test --workspace 2>&1 | tail -3
```

Apply mechanical fixes if clippy warns (backticks in docs, format inlining, redundant clones). Run `cargo fmt --all` if fmt fails. Commit fixes.

- [ ] **Step 2: Mark + tag**

Add `✅` to all task headings.

```bash
git add docs/superpowers/plans/2026-04-26-sradb-rs-slice-8-enrich.md
git commit -m "docs(plan): mark all 8 slice-8 tasks complete"
git tag -a slice-8-enrich -m "Slice 8: LLM enrichment via OpenAI chat completions"
```

---

## Deferred

- Ontology normalization / fuzzy matching against `ontology_reference.json`
- Retry logic with exponential backoff (slice 8b)
- Token budget tracking
- Streaming responses

## Definition of done

- `cargo test --workspace` ≥90 tests
- Wiremock e2e covers happy path + 500 error per-row isolation
- `sradb metadata SRP174132 --detailed --enrich` (with `OPENAI_API_KEY` set) returns rows with populated `enrichment` field
