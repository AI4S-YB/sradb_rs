//! End-to-end test of the enrichment flow against a wiremock `OpenAI` endpoint.

use std::collections::BTreeMap;
use std::time::Duration;

use sradb_core::enrich::{enrich_rows, EnrichConfig};
use sradb_core::model::{Experiment, Library, MetadataRow, Platform, Run, Sample, Study};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture_row() -> MetadataRow {
    let mut attrs = BTreeMap::new();
    attrs.insert("source_name".into(), "liver".into());
    MetadataRow {
        run: Run::default(),
        experiment: Experiment {
            title: Some("RNA-Seq of liver".into()),
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

#[tokio::test]
async fn enrich_rows_populates_enrichment_field() {
    let server = MockServer::start().await;
    let body = r#"{"id":"x","choices":[{"message":{"role":"assistant","content":"{\"organ\":\"liver\",\"tissue\":null,\"anatomical_system\":\"hepatobiliary system\",\"cell_type\":\"hepatocyte\",\"disease\":null,\"sex\":null,\"development_stage\":null,\"assay\":\"RNA-Seq\",\"organism\":\"Homo sapiens\"}"}}]}"#;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let cfg = EnrichConfig {
        api_key: "sk-test".into(),
        base_url: server.uri(),
        model: "gpt-4o-mini".into(),
        temperature: 0.0,
        concurrency: 4,
        max_retries: 0,
        timeout: Duration::from_secs(5),
    };
    let mut rows = vec![fixture_row()];
    enrich_rows(&cfg, &mut rows).await.unwrap();
    let e = rows[0]
        .enrichment
        .as_ref()
        .expect("enrichment should be Some");
    assert_eq!(e.organ.as_deref(), Some("liver"));
    assert_eq!(e.cell_type.as_deref(), Some("hepatocyte"));
    assert_eq!(e.assay.as_deref(), Some("RNA-Seq"));
}

#[tokio::test]
async fn enrich_rows_per_row_failure_isolated() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let cfg = EnrichConfig {
        api_key: "sk-test".into(),
        base_url: server.uri(),
        model: "gpt-4o-mini".into(),
        temperature: 0.0,
        concurrency: 4,
        max_retries: 0,
        timeout: Duration::from_secs(5),
    };
    let mut rows = vec![fixture_row(), fixture_row()];
    enrich_rows(&cfg, &mut rows).await.unwrap();
    assert!(rows[0].enrichment.is_none());
    assert!(rows[1].enrichment.is_none());
}
