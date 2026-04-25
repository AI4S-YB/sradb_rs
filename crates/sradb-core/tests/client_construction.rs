use sradb_core::SraClient;

#[tokio::test]
async fn build_client_with_test_base_urls() {
    let server = sradb_fixtures::mock_server().await;
    let (ncbi, ena) = sradb_fixtures::split_base_urls(&server.uri());
    let client = SraClient::with_base_urls(ncbi, ena).unwrap();
    let cfg = client.config();
    assert!(cfg.ncbi_base_url.starts_with(&server.uri()));
    assert!(cfg.ena_base_url.starts_with(&server.uri()));
}
