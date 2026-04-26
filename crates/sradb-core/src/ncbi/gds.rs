//! NCBI db=gds (GEO Datasets) wrappers.
//!
//! Two functions, both reusing the existing `esearch` and string-based esummary
//! endpoints with `db=gds`.

use serde::Deserialize;

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};

const CONTEXT_ESEARCH: &str = "gds_esearch";
const CONTEXT_ESUMMARY: &str = "gds_esummary";

#[derive(Debug, Deserialize)]
struct EsearchEnvelope {
    esearchresult: EsearchInner,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct EsearchInner {
    count: String,
    #[serde(default, rename = "idlist")]
    ids: Vec<String>,
}

/// Look up the GDS UID list for one accession (GSE/GSM/GPL).
pub async fn gds_esearch_uids(
    http: &HttpClient,
    base_url: &str,
    accession: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>> {
    let url = format!("{base_url}/esearch.fcgi");
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "gds"),
        ("term", accession),
        ("retmode", "json"),
        ("retmax", "20"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    let env: EsearchEnvelope = http.get_json(CONTEXT_ESEARCH, Service::Ncbi, &url, &q).await?;
    Ok(env.esearchresult.ids)
}

/// Fetch the db=gds esummary JSON for one or more UIDs.
pub async fn gds_esummary_by_uids(
    http: &HttpClient,
    base_url: &str,
    uids: &[String],
    api_key: Option<&str>,
) -> Result<String> {
    if uids.is_empty() {
        return Err(SradbError::Parse {
            endpoint: CONTEXT_ESUMMARY,
            message: "empty UID list".into(),
        });
    }
    let id_param = uids.join(",");
    let url = format!("{base_url}/esummary.fcgi");
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "gds"),
        ("id", &id_param),
        ("retmode", "json"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT_ESUMMARY, Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn esearch_returns_uid_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esearch.fcgi"))
            .and(query_param("db", "gds"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"esearchresult":{"count":"1","idlist":["200056924"]}}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let uids = gds_esearch_uids(&http, &server.uri(), "GSE56924", None).await.unwrap();
        assert_eq!(uids, vec!["200056924".to_string()]);
    }

    #[tokio::test]
    async fn esummary_calls_with_id_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esummary.fcgi"))
            .and(query_param("db", "gds"))
            .and(query_param("id", "200056924"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{\"result\":{\"uids\":[]}}"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = gds_esummary_by_uids(&http, &server.uri(), &["200056924".to_string()], None).await.unwrap();
        assert!(body.contains("\"uids\""));
    }
}
