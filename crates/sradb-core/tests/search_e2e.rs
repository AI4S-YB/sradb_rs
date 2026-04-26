//! End-to-end test of the search engine against captured fixtures.

use sradb_core::search::SearchQuery;
use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn search_executes_and_parses_results() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
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
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let query = SearchQuery {
        organism: Some("Homo sapiens".into()),
        strategy: Some("RNA-Seq".into()),
        ..SearchQuery::new()
    };
    let rows = client.search(&query).await.unwrap();
    assert!(!rows.is_empty(), "expected ≥ 1 row from search");
    for row in &rows {
        assert_eq!(row.sample.organism_name.as_deref(), Some("Homo sapiens"));
        assert_eq!(row.experiment.library.strategy.as_deref(), Some("RNA-Seq"));
    }
}

#[tokio::test]
async fn empty_query_returns_error() {
    let server = MockServer::start().await;
    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let err = client.search(&SearchQuery::new()).await.unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("empty search query"), "unexpected: {msg}");
}

#[tokio::test]
async fn esearch_term_includes_orgn_and_stra_qualifiers() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body = std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("term", "\"Homo sapiens\"[ORGN] AND \"RNA-Seq\"[STRA]"))
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
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let query = SearchQuery {
        organism: Some("Homo sapiens".into()),
        strategy: Some("RNA-Seq".into()),
        ..SearchQuery::new()
    };
    let rows = client.search(&query).await.unwrap();
    assert!(!rows.is_empty());
}
