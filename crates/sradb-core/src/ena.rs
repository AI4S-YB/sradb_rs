//! ENA portal API client (filereport endpoint).

use crate::error::Result;
use crate::http::{HttpClient, Service};

const CONTEXT: &str = "ena_filereport";

/// Fetch the ENA filereport TSV for one run accession.
///
/// Returns the raw TSV body. Use `parse::ena_filereport::parse` to decode.
pub async fn fetch_filereport(
    http: &HttpClient,
    base_url: &str,
    run_accession: &str,
) -> Result<String> {
    let url = format!("{base_url}/portal/api/filereport");
    http.get_text(
        CONTEXT,
        Service::Ena,
        &url,
        &[
            ("accession", run_accession),
            ("result", "read_run"),
            ("fields", "fastq_ftp,fastq_md5,fastq_bytes,fastq_aspera"),
            ("format", "tsv"),
        ],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn filereport_returns_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/portal/api/filereport"))
            .and(query_param("accession", "SRR1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("run_accession\tfastq_ftp\nSRR1\tx.fastq.gz"))
            .mount(&server)
            .await;

        let http = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
        let body = fetch_filereport(&http, &server.uri(), "SRR1").await.unwrap();
        assert!(body.contains("SRR1"));
    }
}
