//! End-to-end test of HTTP download with Range resume.

use std::time::Duration;

use sradb_core::download::{download_one, download_plan, DownloadItem, DownloadPlan};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn downloads_a_small_file() {
    let server = MockServer::start().await;
    let body = b"hello world".to_vec();
    Mock::given(method("GET"))
        .and(path("/foo.txt"))
        .and(header("accept-encoding", "identity"))
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
async fn resumes_after_interrupted_body() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let req = read_http_request(&mut stream).await;
        let req_lower = req.to_ascii_lowercase();
        assert!(req_lower.contains("accept-encoding: identity"));
        assert!(!req_lower.contains("\r\nrange:"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello ")
            .await
            .unwrap();
        stream.shutdown().await.unwrap();

        let (mut stream, _) = listener.accept().await.unwrap();
        let req = read_http_request(&mut stream).await;
        let req_lower = req.to_ascii_lowercase();
        assert!(req_lower.contains("accept-encoding: identity"));
        assert!(req_lower.contains("range: bytes=6-"));
        stream
            .write_all(
                b"HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nContent-Range: bytes 6-10/11\r\nConnection: close\r\n\r\nworld",
            )
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
    });

    let tmp = TempDir::new().unwrap();
    let dest = tmp.path().join("flaky.txt");
    let item = DownloadItem {
        url: format!("http://{addr}/flaky.txt"),
        dest_path: dest.clone(),
        expected_size: Some(11),
    };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .http1_only()
        .build()
        .unwrap();
    let written = download_one(&http, &item).await.unwrap();
    assert_eq!(written, 11);
    assert_eq!(std::fs::read(&dest).unwrap(), b"hello world");
    server.await.unwrap();
}

async fn read_http_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0_u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
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
