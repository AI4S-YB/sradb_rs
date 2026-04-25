//! Async HTTP wrapper shared across all backends.
//!
//! Wraps `reqwest::Client` with:
//! - exponential-backoff retry on transient HTTP/network errors via `reqwest_retry`
//! - per-service token-bucket rate limiting via `governor`
//! - a small surface (`get_bytes`, `get_text`, `get_json`) so callers don't see middleware machinery

use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::RetryTransientMiddleware;

use crate::error::{Result, SradbError};

/// Service whose rate-limit bucket a request should be charged against.
#[derive(Debug, Clone, Copy)]
pub enum Service {
    Ncbi,
    Ena,
    Other,
}

/// Per-service rate-limited HTTP client.
#[derive(Clone)]
pub struct HttpClient {
    inner: ClientWithMiddleware,
    ncbi_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    ena_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient").finish_non_exhaustive()
    }
}

impl HttpClient {
    /// Build a client.
    /// - `ncbi_rps`: requests/sec for NCBI eUtils (3 without `api_key`, 10 with).
    /// - `ena_rps`: requests/sec for ENA. Default 8.
    /// - `max_retries`: retry attempts for transient failures.
    pub fn new(ncbi_rps: u32, ena_rps: u32, max_retries: u32, timeout: Duration) -> Result<Self> {
        let base = reqwest::Client::builder()
            .timeout(timeout)
            .user_agent(default_user_agent())
            .build()
            .map_err(|source| SradbError::Http {
                endpoint: "client_init",
                source,
            })?;

        let policy = ExponentialBackoff::builder()
            .retry_bounds(Duration::from_millis(500), Duration::from_secs(30))
            .build_with_max_retries(max_retries);

        let inner = ClientBuilder::new(base)
            .with(RetryTransientMiddleware::new_with_policy(policy))
            .build();

        let ncbi_q = Quota::per_second(NonZeroU32::new(ncbi_rps.max(1)).unwrap());
        let ena_q = Quota::per_second(NonZeroU32::new(ena_rps.max(1)).unwrap());

        Ok(Self {
            inner,
            ncbi_limiter: Arc::new(RateLimiter::direct(ncbi_q)),
            ena_limiter: Arc::new(RateLimiter::direct(ena_q)),
        })
    }

    async fn wait_quota(&self, service: Service) {
        match service {
            Service::Ncbi => self.ncbi_limiter.until_ready().await,
            Service::Ena => self.ena_limiter.until_ready().await,
            Service::Other => {}
        }
    }

    /// GET → bytes. `endpoint` is a static label used in errors.
    pub async fn get_bytes(
        &self,
        endpoint: &'static str,
        service: Service,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<bytes::Bytes> {
        self.wait_quota(service).await;
        let resp = self
            .inner
            .get(url)
            .query(query)
            .send()
            .await
            .map_err(|source| SradbError::HttpMiddleware { endpoint, source })?;
        check_status(endpoint, &resp)?;
        resp.bytes()
            .await
            .map_err(|source| SradbError::Http { endpoint, source })
    }

    pub async fn get_text(
        &self,
        endpoint: &'static str,
        service: Service,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<String> {
        let bytes = self.get_bytes(endpoint, service, url, query).await?;
        String::from_utf8(bytes.to_vec()).map_err(|e| SradbError::Parse {
            endpoint,
            message: format!("response is not valid UTF-8: {e}"),
        })
    }

    pub async fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &'static str,
        service: Service,
        url: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        let bytes = self.get_bytes(endpoint, service, url, query).await?;
        serde_json::from_slice(&bytes).map_err(|source| SradbError::Json {
            context: endpoint,
            source,
        })
    }
}

fn check_status(endpoint: &'static str, resp: &reqwest::Response) -> Result<()> {
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(SradbError::NotFound(format!("{endpoint}: {}", resp.url())));
    }
    Err(SradbError::Parse {
        endpoint,
        message: format!("unexpected status {status} from {}", resp.url()),
    })
}

fn default_user_agent() -> String {
    format!("sradb-rs/{}", env!("CARGO_PKG_VERSION"))
}
