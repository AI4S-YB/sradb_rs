//! High-level client. Slice 1 only stands up the shell + config; later slices
//! add the metadata/convert/search/download methods on top.

use std::time::Duration;

use crate::error::Result;
use crate::http::HttpClient;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub api_key: Option<String>,
    pub email: Option<String>,
    pub user_agent: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub ncbi_base_url: String,
    pub ena_base_url: String,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            api_key: std::env::var("NCBI_API_KEY").ok().filter(|s| !s.is_empty()),
            email: std::env::var("NCBI_EMAIL").ok().filter(|s| !s.is_empty()),
            user_agent: None,
            timeout: Duration::from_secs(30),
            max_retries: 5,
            ncbi_base_url: "https://eutils.ncbi.nlm.nih.gov/entrez/eutils".into(),
            ena_base_url: "https://www.ebi.ac.uk/ena".into(),
        }
    }
}

impl ClientConfig {
    /// True when an NCBI `api_key` is configured (raises rate limit from 3rps to 10rps).
    #[must_use]
    pub fn has_api_key(&self) -> bool {
        self.api_key.as_deref().is_some_and(|s| !s.is_empty())
    }
}

#[derive(Clone)]
pub struct SraClient {
    pub(crate) http: HttpClient,
    pub(crate) cfg: ClientConfig,
}

impl std::fmt::Debug for SraClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SraClient")
            .field("has_api_key", &self.cfg.has_api_key())
            .field("ncbi_base_url", &self.cfg.ncbi_base_url)
            .field("ena_base_url", &self.cfg.ena_base_url)
            .finish_non_exhaustive()
    }
}

impl SraClient {
    pub fn new() -> Result<Self> {
        Self::with_config(ClientConfig::default())
    }

    pub fn with_config(cfg: ClientConfig) -> Result<Self> {
        let ncbi_rps = if cfg.has_api_key() { 10 } else { 3 };
        let ena_rps = 8;
        let http = HttpClient::new(ncbi_rps, ena_rps, cfg.max_retries, cfg.timeout)?;
        Ok(Self { http, cfg })
    }

    #[must_use]
    pub fn config(&self) -> &ClientConfig {
        &self.cfg
    }

    /// Convenience: same as `with_config` but overriding both base URLs.
    /// Used in tests to point the client at a wiremock server.
    pub fn with_base_urls(ncbi: impl Into<String>, ena: impl Into<String>) -> Result<Self> {
        let cfg = ClientConfig {
            ncbi_base_url: ncbi.into(),
            ena_base_url: ena.into(),
            ..ClientConfig::default()
        };
        Self::with_config(cfg)
    }

    /// Fetch metadata for one accession.
    pub async fn metadata(
        &self,
        accession: &str,
        opts: &crate::model::MetadataOpts,
    ) -> Result<Vec<crate::model::MetadataRow>> {
        crate::metadata::fetch_metadata(
            &self.http,
            &self.cfg.ncbi_base_url,
            &self.cfg.ena_base_url,
            self.cfg.api_key.as_deref(),
            accession,
            opts,
        )
        .await
    }

    /// Fetch metadata for many accessions concurrently. The returned vec is
    /// in input order; each element is the per-accession result (success or
    /// error). Failures of one accession do not abort the others.
    pub async fn metadata_many(
        &self,
        accessions: &[String],
        opts: &crate::model::MetadataOpts,
    ) -> Vec<Result<Vec<crate::model::MetadataRow>>> {
        let futures = accessions.iter().map(|a| self.metadata(a, opts));
        futures::future::join_all(futures).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_pulls_env() {
        std::env::set_var("NCBI_API_KEY", "abc");
        let cfg = ClientConfig::default();
        assert!(cfg.has_api_key());
        std::env::remove_var("NCBI_API_KEY");
    }

    #[test]
    fn empty_env_var_is_treated_as_unset() {
        std::env::set_var("NCBI_API_KEY", "");
        let cfg = ClientConfig::default();
        assert!(!cfg.has_api_key());
        std::env::remove_var("NCBI_API_KEY");
    }

    #[test]
    fn client_constructs() {
        let c = SraClient::new().unwrap();
        assert!(!c.config().ncbi_base_url.is_empty());
    }
}
