//! End-to-end test of HTTP download with Range resume.

use std::time::Duration;

use sradb_core::download::{download_one, download_plan, DownloadItem, DownloadPlan};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn downloads_a_small_file() {
    let server = MockServer::start().await;
    let body = b"hello world".to_vec();
    Mock::given(method("GET"))
        .and(path("/foo.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("foo.txt");
    let item = DownloadItem {
        url: format!("{}/foo.txt", server.uri()),
        dest_path: dest.clone(),
        expected_size: Some(body.len() as u64),
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let written = download_one(&http, &item).await.unwrap();
    assert_eq!(written, body.len() as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), body);
}

#[tokio::test]
async fn skips_existing_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"x".to_vec()))
        .mount(&server)
        .await;

    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("foo.txt");
    std::fs::write(&dest, b"already here").unwrap();

    let item = DownloadItem {
        url: format!("{}/foo.txt", server.uri()),
        dest_path: dest.clone(),
        expected_size: None,
    };
    let http = reqwest::Client::builder().build().unwrap();
    let written = download_one(&http, &item).await.unwrap();
    assert_eq!(written, 0);
    assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
}

#[tokio::test]
async fn parallel_plan_executes_all() {
    let server = MockServer::start().await;
    for i in 0..5 {
        Mock::given(method("GET"))
            .and(path(format!("/{i}.txt")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 100]))
            .mount(&server)
            .await;
    }

    let tmp = TempDir::new().unwrap();
    let plan = DownloadPlan {
        items: (0..5)
            .map(|i| DownloadItem {
                url: format!("{}/{i}.txt", server.uri()),
                dest_path: tmp.path().join(format!("{i}.txt")),
                expected_size: Some(100),
            })
            .collect(),
    };
    let http = reqwest::Client::builder().build().unwrap();
    let report = download_plan(&http, &plan, 2).await;
    assert_eq!(report.completed, 5);
    assert_eq!(report.failed, 0);
    for i in 0..5 {
        let p = tmp.path().join(format!("{i}.txt"));
        assert!(p.exists());
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 100);
    }
}
