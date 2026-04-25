//! NCBI esearch wrapper.

use serde::Deserialize;

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};
use crate::ncbi::EsearchResult;

const CONTEXT: &str = "esearch";

#[derive(Debug, Deserialize)]
struct Envelope {
    esearchresult: Inner,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct Inner {
    count: String,
    #[serde(rename = "webenv")]
    webenv: String,
    #[serde(rename = "querykey")]
    query_key: String,
    #[serde(default, rename = "idlist")]
    ids: Vec<String>,
}

/// Run NCBI esearch with `usehistory=y` and parse the JSON response.
pub async fn esearch(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    term: &str,
    api_key: Option<&str>,
    retmax: u32,
) -> Result<EsearchResult> {
    let url = format!("{base_url}/esearch.fcgi");
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("term", term),
        ("retmode", "json"),
        ("retmax", &retmax_s),
        ("usehistory", "y"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    let env: Envelope = http.get_json(CONTEXT, Service::Ncbi, &url, &q).await?;
    let count = env
        .esearchresult
        .count
        .parse::<u64>()
        .map_err(|e| SradbError::Parse {
            endpoint: CONTEXT,
            message: format!("count `{}` not a u64: {e}", env.esearchresult.count),
        })?;
    Ok(EsearchResult {
        count,
        webenv: env.esearchresult.webenv,
        query_key: env.esearchresult.query_key,
        ids: env.esearchresult.ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn esearch_parses_typical_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esearch.fcgi"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"header":{"type":"esearch","version":"0.3"},"esearchresult":{"count":"10","retmax":"500","retstart":"0","querykey":"1","webenv":"MCID_abc123","idlist":["1","2","3"]}}"#,
            ))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let result = esearch(&http, &server.uri(), "sra", "SRP174132", None, 500)
            .await
            .unwrap();
        assert_eq!(result.count, 10);
        assert_eq!(result.webenv, "MCID_abc123");
        assert_eq!(result.query_key, "1");
        assert_eq!(result.ids, vec!["1".to_string(), "2".into(), "3".into()]);
    }
}
