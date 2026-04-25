//! End-to-end test of `SraClient::metadata` against captured fixtures served by wiremock.

use sradb_core::{ClientConfig, MetadataOpts, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn metadata_srp174132_against_fixtures() {
    let esearch_body = std::fs::read_to_string(
        sradb_fixtures::workspace_root().join("tests/data/ncbi/esearch_SRP174132.json"),
    )
    .expect("run `cargo run -p capture-fixtures -- save-esearch SRP174132` first");
    let esummary_body = std::fs::read_to_string(
        sradb_fixtures::workspace_root().join("tests/data/ncbi/esummary_SRP174132.xml"),
    )
    .expect("run `cargo run -p capture-fixtures -- save-esummary SRP174132` first");

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("term", "SRP174132"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esummary_body))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: format!("{}/ena", server.uri()),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let mut rows = client.metadata("SRP174132", &MetadataOpts::new()).await.unwrap();
    rows.sort_by(|a, b| a.run.accession.cmp(&b.run.accession));

    assert!(!rows.is_empty(), "expected at least 1 row");
    for r in &rows {
        assert_eq!(r.study.accession, "SRP174132", "study accession should match");
        assert!(r.run.accession.starts_with("SRR"), "run acc: {}", r.run.accession);
        assert!(r.experiment.accession.starts_with("SRX"));
        assert!(r.sample.accession.starts_with("SRS"));
        assert_eq!(r.sample.organism_name.as_deref(), Some("Homo sapiens"));
        assert_eq!(r.sample.organism_taxid, Some(9606));
        assert_eq!(r.experiment.library.strategy.as_deref(), Some("RNA-Seq"));
    }

    insta::assert_json_snapshot!("metadata_srp174132", rows, {
        "[].run.published" => "[date]",
    });
}
