//! End-to-end test of the detailed metadata path against captured fixtures.

use sradb_core::{ClientConfig, MetadataOpts, SraClient};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn metadata_detailed_srp174132() {
    let workspace = sradb_fixtures::workspace_root();
    let esearch_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/esearch_SRP174132.json"))
            .expect("esearch fixture");
    let esummary_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/esummary_SRP174132.xml"))
            .expect("esummary fixture");
    let runinfo_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/efetch_runinfo_SRP174132.csv"))
            .expect("runinfo fixture");
    let xml_body =
        std::fs::read_to_string(workspace.join("tests/data/ncbi/efetch_xml_SRP174132.xml"))
            .expect("efetch xml fixture");
    let ena_body =
        std::fs::read_to_string(workspace.join("tests/data/ena/filereport_SRR8361601.tsv"))
            .expect("ena fixture");

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
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .and(query_param("rettype", "runinfo"))
        .respond_with(ResponseTemplate::new(200).set_body_string(runinfo_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/efetch.fcgi"))
        .and(query_param("rettype", "full"))
        .respond_with(ResponseTemplate::new(200).set_body_string(xml_body))
        .mount(&server)
        .await;
    // ENA: only one fixture, but the orchestrator fans out per-run.
    // For the matched accession, return the captured TSV.
    // For other accessions, return an empty body (parser yields empty rows).
    Mock::given(method("GET"))
        .and(path("/portal/api/filereport"))
        .and(query_param("accession", "SRR8361601"))
        .respond_with(ResponseTemplate::new(200).set_body_string(ena_body))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/portal/api/filereport"))
        .respond_with(ResponseTemplate::new(200).set_body_string(""))
        .mount(&server)
        .await;

    let cfg = ClientConfig {
        ncbi_base_url: server.uri(),
        ena_base_url: server.uri(),
        ..ClientConfig::default()
    };
    let client = SraClient::with_config(cfg).unwrap();
    let opts = MetadataOpts {
        detailed: true,
        enrich: false,
        page_size: 500,
    };
    let mut rows = client.metadata("SRP174132", &opts).await.unwrap();
    rows.sort_by(|a, b| a.run.accession.cmp(&b.run.accession));

    assert!(!rows.is_empty(), "expected ≥ 1 row");

    // Sample attributes populated for every row (from EXPERIMENT_PACKAGE_SET).
    for r in &rows {
        assert!(
            !r.sample.attributes.is_empty(),
            "{} should have sample attrs",
            r.run.accession
        );
    }

    // The single ENA-fixture run should have fastq URLs.
    let r = rows
        .iter()
        .find(|r| r.run.accession == "SRR8361601")
        .expect("SRR8361601 must be present");
    assert!(
        !r.run.urls.ena_fastq_ftp.is_empty(),
        "SRR8361601 should have ENA fastq FTP URLs"
    );
    assert!(
        !r.run.urls.ena_fastq_http.is_empty(),
        "SRR8361601 should have ENA fastq HTTP URLs"
    );

    // Some run should have at least one of NCBI/S3/GS URLs from the EXPERIMENT_PACKAGE_SET.
    let any_dl = rows.iter().any(|r| {
        r.run.urls.ncbi_sra.is_some() || r.run.urls.s3.is_some() || r.run.urls.gs.is_some()
    });
    assert!(
        any_dl,
        "at least one run should have a download URL from SRAFiles"
    );
}
