//! End-to-end test of the convert engine against captured fixtures.

use sradb_core::accession::{Accession, AccessionKind};
use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn srp_to_srx_via_metadata_projection() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("db", "sra"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .and(query_param("db", "sra"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esummary_body))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();

    let input: Accession = "SRP174132".parse().unwrap();
    let result = client.convert(&input, AccessionKind::Srx).await.unwrap();

    // SRP174132 has 10 experiments → 10 unique SRX accessions.
    assert_eq!(result.len(), 10, "expected 10 SRX accessions, got {}: {:?}", result.len(), result);
    for acc in &result {
        assert_eq!(acc.kind, AccessionKind::Srx);
        assert!(acc.raw.starts_with("SRX"), "{}", acc.raw);
    }
}

#[tokio::test]
async fn gse_to_srp_via_gds_lookup() {
    let workspace = sradb_fixtures::workspace_root();
    let gds_esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/gds_esearch_GSE56924.json")).unwrap();
    let gds_esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/gds_esummary_GSE56924.json")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("db", "gds"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gds_esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .and(query_param("db", "gds"))
        .respond_with(ResponseTemplate::new(200).set_body_string(gds_esummary_body))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();

    let input: Accession = "GSE56924".parse().unwrap();
    let result = client.convert(&input, AccessionKind::Srp).await.unwrap();

    assert!(!result.is_empty(), "expected at least one SRP from GSE56924");
    for acc in &result {
        assert_eq!(acc.kind, AccessionKind::Srp);
        assert!(acc.raw.starts_with("SRP"), "{}", acc.raw);
    }
}

#[tokio::test]
async fn identity_returns_input() {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg).unwrap();
    let input: Accession = "SRP174132".parse().unwrap();
    let result = client.convert(&input, AccessionKind::Srp).await.unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].raw, "SRP174132");
}

#[tokio::test]
async fn unsupported_conversion_errors() {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg).unwrap();
    let input: Accession = "SRP174132".parse().unwrap();
    let err = client.convert(&input, AccessionKind::Pmid).await.unwrap_err();
    assert!(matches!(err, sradb_core::SradbError::UnsupportedConversion { .. }));
}
