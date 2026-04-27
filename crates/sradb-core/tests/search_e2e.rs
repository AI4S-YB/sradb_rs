//! End-to-end test of the search engine against captured fixtures.

use sradb_core::search::SearchQuery;
use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn search_executes_and_parses_results() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

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
    let esearch_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json")).unwrap();
    let esummary_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param(
            "term",
            "\"Homo sapiens\"[ORGN] AND \"RNA-Seq\"[STRA]",
        ))
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

#[tokio::test]
async fn ena_search_parses_tsv_response() {
    let body = "run_accession\texperiment_accession\tsample_accession\tstudy_accession\tscientific_name\tlibrary_strategy\tlibrary_source\tlibrary_selection\tlibrary_layout\tinstrument_platform\tinstrument_model\tread_count\tbase_count\tstudy_title\n\
                SRR1\tSRX1\tSRS1\tSRP1\tHomo sapiens\tRNA-Seq\tTRANSCRIPTOMIC\tcDNA\tPAIRED\tILLUMINA\tIllumina HiSeq 2000\t1234\t999999\tStudy A\n\
                SRR2\tSRX2\tSRS2\tSRP2\tMus musculus\tWGS\tGENOMIC\tRANDOM\tSINGLE\tILLUMINA\tIllumina HiSeq 4000\t\t\t\n";

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/portal/api/search"))
        .and(query_param("result", "read_run"))
        .and(query_param("format", "tsv"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
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
    let hits = client.search_ena(&query).await.unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].run_accession, "SRR1");
    assert_eq!(hits[0].read_count, Some(1234));
    assert_eq!(hits[0].scientific_name.as_deref(), Some("Homo sapiens"));
    assert_eq!(hits[1].read_count, None);
}

#[tokio::test]
async fn ena_search_query_param_includes_filters() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/portal/api/search"))
        .and(query_param(
            "query",
            "tax_name=\"Homo sapiens\" AND library_strategy=\"RNA-Seq\"",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string("run_accession\nSRR_only\n"))
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
    let hits = client.search_ena(&query).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].run_accession, "SRR_only");
}

#[tokio::test]
async fn geo_search_uses_db_gds_and_projects_extrelations() {
    let esearch_body = r#"{"header":{"type":"esearch"},"esearchresult":{"count":"1","retmax":"20","retstart":"0","querykey":"1","webenv":"X","idlist":["200056924"]}}"#;
    let esummary_body = r#"{"header":{"type":"esummary"},"result":{"uids":["200056924"],
"200056924":{"uid":"200056924","accession":"GSE56924","entrytype":"GSE","n_samples":96,
"samples":[],
"extrelations":[{"relationtype":"SRA","targetobject":"SRP041298"}]}}}"#;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/esearch.fcgi"))
        .and(query_param("db", "gds"))
        .respond_with(ResponseTemplate::new(200).set_body_string(esearch_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/esummary.fcgi"))
        .and(query_param("db", "gds"))
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
        query: Some("GSE56924".into()),
        ..SearchQuery::new()
    };
    let hits = client.search_geo(&query).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].accession, "GSE56924");
    assert_eq!(hits[0].entry_type, "GSE");
    assert_eq!(hits[0].n_samples, Some(96));
    assert_eq!(hits[0].sra_accession.as_deref(), Some("SRP041298"));
}
