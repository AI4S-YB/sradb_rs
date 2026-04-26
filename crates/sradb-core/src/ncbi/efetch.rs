//! NCBI efetch wrappers.

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT_RUNINFO: &str = "efetch_runinfo";
const CONTEXT_XML: &str = "efetch_xml";

/// Fetch one page of efetch retmode=runinfo (CSV) using a (`WebEnv`, `query_key`) handle.
#[allow(clippy::too_many_arguments)]
pub async fn efetch_runinfo_with_history(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    webenv: &str,
    query_key: &str,
    retstart: u32,
    retmax: u32,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/efetch.fcgi");
    let retstart_s = retstart.to_string();
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", &retstart_s),
        ("retmax", &retmax_s),
        ("rettype", "runinfo"),
        ("retmode", "csv"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT_RUNINFO, Service::Ncbi, &url, &q).await
}

/// Fetch one page of efetch retmode=xml (full EXPERIMENT_PACKAGE_SET) using a (`WebEnv`, `query_key`) handle.
#[allow(clippy::too_many_arguments)]
pub async fn efetch_full_xml_with_history(
    http: &HttpClient,
    base_url: &str,
    db: &str,
    webenv: &str,
    query_key: &str,
    retstart: u32,
    retmax: u32,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/efetch.fcgi");
    let retstart_s = retstart.to_string();
    let retmax_s = retmax.to_string();
    let mut q: Vec<(&str, &str)> = vec![
        ("db", db),
        ("WebEnv", webenv),
        ("query_key", query_key),
        ("retstart", &retstart_s),
        ("retmax", &retmax_s),
        ("rettype", "full"),
        ("retmode", "xml"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text(CONTEXT_XML, Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn runinfo_calls_efetch_with_csv_args() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/efetch.fcgi"))
            .and(query_param("rettype", "runinfo"))
            .and(query_param("retmode", "csv"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Run,bases\nSRR1,100\n"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = efetch_runinfo_with_history(&http, &server.uri(), "sra", "WE", "QK", 0, 500, None)
            .await.unwrap();
        assert!(body.contains("Run,bases"));
    }

    #[tokio::test]
    async fn full_xml_calls_efetch_with_xml_args() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/efetch.fcgi"))
            .and(query_param("rettype", "full"))
            .and(query_param("retmode", "xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<EXPERIMENT_PACKAGE_SET/>"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = efetch_full_xml_with_history(&http, &server.uri(), "sra", "WE", "QK", 0, 500, None)
            .await.unwrap();
        assert_eq!(body, "<EXPERIMENT_PACKAGE_SET/>");
    }
}
