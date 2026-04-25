use std::time::Duration;

use sradb_core::http::{HttpClient, Service};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_text_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/hello"))
        .respond_with(ResponseTemplate::new(200).set_body_string("hi"))
        .mount(&server)
        .await;

    let client = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
    let body = client
        .get_text("test", Service::Other, &format!("{}/hello", server.uri()), &[])
        .await
        .unwrap();
    assert_eq!(body, "hi");
}

#[tokio::test]
async fn retries_on_500_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/x"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = HttpClient::new(10, 10, 3, Duration::from_secs(5)).unwrap();
    let body = client
        .get_text("test", Service::Other, &format!("{}/x", server.uri()), &[])
        .await
        .unwrap();
    assert_eq!(body, "ok");
}

#[tokio::test]
async fn returns_not_found_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = HttpClient::new(10, 10, 0, Duration::from_secs(5)).unwrap();
    let err = client
        .get_text("test", Service::Other, &format!("{}/missing", server.uri()), &[])
        .await
        .unwrap_err();
    assert!(matches!(err, sradb_core::SradbError::NotFound(_)));
}
