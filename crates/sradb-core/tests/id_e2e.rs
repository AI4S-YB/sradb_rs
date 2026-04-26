//! End-to-end test of identifier extraction against wiremock.

use sradb_core::{ClientConfig, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PMC_BODY: &str = r#"<article>
The processed data are deposited at GEO under accession GSE253406 and
the raw reads at SRA under SRP484103. BioProject PRJNA1058002 covers
both. Sample-level deposits include GSM12345 and GSM12346.
</article>"#;

#[tokio::test]
async fn from_pmc_extracts_identifiers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .and(query_param("db", "pmc"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PMC_BODY))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let set = client.identifiers_from_pmc("PMC10802650").await.unwrap();
    assert_eq!(set.pmc_id.as_deref(), Some("PMC10802650"));
    assert_eq!(set.gse_ids, vec!["GSE253406".to_string()]);
    assert_eq!(set.srp_ids, vec!["SRP484103".to_string()]);
    assert_eq!(set.prjna_ids, vec!["PRJNA1058002".to_string()]);
    assert_eq!(
        set.gsm_ids,
        vec!["GSM12345".to_string(), "GSM12346".to_string()]
    );
}

#[tokio::test]
async fn from_pmid_chains_through_elink_and_efetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/elink.fcgi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            r#"{"linksets":[{"linksetdbs":[{"linkname":"pubmed_pmc","links":["10802650"]}]}]}"#,
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PMC_BODY))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let set = client.identifiers_from_pmid(39_528_918).await.unwrap();
    assert_eq!(set.pmid, Some(39_528_918));
    assert_eq!(set.pmc_id.as_deref(), Some("PMC10802650"));
    assert_eq!(set.gse_ids, vec!["GSE253406".to_string()]);
}

#[tokio::test]
async fn from_pmc_invalid_input_errors() {
    let server = MockServer::start().await;
    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    assert!(client.identifiers_from_pmc("not-a-pmc").await.is_err());
    assert!(client.identifiers_from_pmc("PMC").await.is_err());
}
