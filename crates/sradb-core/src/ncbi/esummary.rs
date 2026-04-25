//! NCBI esummary wrapper.

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT: &str = "esummary";

/// Fetch one page of esummary results using a (`WebEnv`, `query_key`) handle.
/// Returns the raw response body (XML by default for db=sra).
pub async fn esummary_with_history(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    webenv: &str,
    query_key: &str,
    retstart: u32,
    retmax: u32,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/esummary.fcgi");
    let retstart_s = retstart.to_string();
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", &retstart_s),
        ("retmax", &retmax_s),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT, Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn esummary_returns_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/esummary.fcgi"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<eSummaryResult/>"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = esummary_with_history(&http, &server.uri(), "sra", "WE", "QK", 0, 500, None)
            .await
            .unwrap();
        assert_eq!(body, "<eSummaryResult/>");
    }
}
