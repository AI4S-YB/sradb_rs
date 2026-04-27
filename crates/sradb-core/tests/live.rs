//! Live integration tests against real SRA / ENA / GEO endpoints.
//!
//! Gated behind the `live` feature so the default `cargo test` stays hermetic.
//! Run manually with:
//!
//! ```sh
//! cargo test -p sradb-core --features live --test live -- --test-threads=1
//! ```

#![cfg(feature = "live")]

use sradb_core::search::SearchQuery;
use sradb_core::{ClientConfig, MetadataOpts, SraClient};

fn live_client() -> SraClient {
    SraClient::with_config(ClientConfig::default()).expect("client builds")
}

#[tokio::test]
async fn metadata_srp174132_returns_rows() {
    let client = live_client();
    let opts = MetadataOpts {
        detailed: false,
        enrich: false,
        page_size: 500,
    };
    let rows = client.metadata("SRP174132", &opts).await.unwrap();
    assert!(!rows.is_empty(), "expected rows for SRP174132");
    for row in &rows {
        assert!(row.run.accession.starts_with("SRR"));
        assert_eq!(row.study.accession, "SRP174132");
    }
}

#[tokio::test]
async fn search_sra_homo_rnaseq_returns_some_rows() {
    let client = live_client();
    let q = SearchQuery {
        organism: Some("Homo sapiens".into()),
        strategy: Some("RNA-Seq".into()),
        max: 3,
        ..SearchQuery::new()
    };
    let rows = client.search(&q).await.unwrap();
    assert!(!rows.is_empty(), "expected ≥ 1 SRA result");
}

#[tokio::test]
async fn search_ena_homo_rnaseq_returns_some_hits() {
    let client = live_client();
    let q = SearchQuery {
        organism: Some("Homo sapiens".into()),
        strategy: Some("RNA-Seq".into()),
        max: 3,
        ..SearchQuery::new()
    };
    let hits = client.search_ena(&q).await.unwrap();
    assert!(!hits.is_empty(), "expected ≥ 1 ENA hit");
    for h in &hits {
        assert!(h.run_accession.starts_with("SRR") || h.run_accession.starts_with("ERR"));
    }
}

#[tokio::test]
async fn search_geo_gse56924_returns_record() {
    let client = live_client();
    let q = SearchQuery {
        query: Some("GSE56924".into()),
        max: 5,
        ..SearchQuery::new()
    };
    let hits = client.search_geo(&q).await.unwrap();
    assert!(hits.iter().any(|h| h.accession == "GSE56924"));
}

#[tokio::test]
async fn identifiers_from_pmid_returns_set() {
    let client = live_client();
    // PMID 24349042 → known to link to SRP/GSE accessions.
    let set = client.identifiers_from_pmid(24_349_042).await.unwrap();
    assert_eq!(set.pmid, Some(24_349_042));
}
