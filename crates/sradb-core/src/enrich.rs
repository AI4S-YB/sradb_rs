//! LLM-based metadata enrichment via an OpenAI-compatible chat completions API.
//!
//! For each `MetadataRow`, construct a prompt from the sample/experiment titles
//! and SAMPLE_ATTRIBUTES bag, send to the configured chat-completions endpoint
//! with a strict JSON schema response format, and decode the result into the
//! 9-field `Enrichment` struct.

use std::sync::Arc;
use std::time::Duration;

use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

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

/// Parse the OpenAI response body into an `Enrichment`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Experiment, Library, Platform, Run, Sample, Study};
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
