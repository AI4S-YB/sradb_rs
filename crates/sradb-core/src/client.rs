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

    /// Convert an accession to one or more accessions of `to_kind`.
    /// Returns an empty vec if the input maps to nothing; returns `Err` for unsupported pairs.
    pub async fn convert(
        &self,
        input: &crate::accession::Accession,
        to_kind: crate::accession::AccessionKind,
    ) -> Result<Vec<crate::accession::Accession>> {
        crate::convert::convert_one(
            &self.http,
            &self.cfg.ncbi_base_url,
            &self.cfg.ena_base_url,
            self.cfg.api_key.as_deref(),
            input,
            to_kind,
        )
        .await
    }

    /// Like `convert` but follows up with a metadata fetch for each result.
    /// Useful when the caller wants both the converted accessions and full
    /// metadata in a single call.
    pub async fn convert_detailed(
        &self,
        input: &crate::accession::Accession,
        to_kind: crate::accession::AccessionKind,
    ) -> Result<Vec<crate::model::MetadataRow>> {
        let converted = self.convert(input, to_kind).await?;
        let opts = crate::model::MetadataOpts {
            detailed: false,
            enrich: false,
            page_size: 500,
        };
        let mut rows: Vec<crate::model::MetadataRow> = Vec::new();
        for acc in &converted {
            let part = self.metadata(&acc.raw, &opts).await?;
            rows.extend(part);
        }
        Ok(rows)
    }

    /// Search SRA via Entrez query terms built from a `SearchQuery`.
    pub async fn search(
        &self,
        query: &crate::search::SearchQuery,
    ) -> Result<Vec<crate::model::MetadataRow>> {
        crate::search::search(
            &self.http,
            &self.cfg.ncbi_base_url,
            self.cfg.api_key.as_deref(),
            query,
        )
        .await
    }

    /// Download a list of `DownloadItem`s with bounded parallelism.
    pub async fn download(
        &self,
        plan: &crate::download::DownloadPlan,
        parallelism: usize,
    ) -> crate::download::DownloadReport {
        // The `http` field is our reqwest-middleware wrapper; for raw streaming
        // downloads we use a fresh `reqwest::Client` with the same defaults.
        let raw = reqwest::Client::builder()
            .timeout(self.cfg.timeout)
            .user_agent(format!("sradb-rs/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client build");
        crate::download::download_plan(&raw, plan, parallelism).await
    }

    /// Enrich a list of metadata rows in place using LLM-extracted fields.
    /// `EnrichConfig` is typically built via `EnrichConfig::from_env()`.
    pub async fn enrich_rows(
        &self,
        cfg: &crate::enrich::EnrichConfig,
        rows: &mut [crate::model::MetadataRow],
    ) -> Result<()> {
        crate::enrich::enrich_rows(cfg, rows).await
    }

    /// Extract database identifiers from a `PubMed` PMID.
    pub async fn identifiers_from_pmid(
        &self,
        pmid: u64,
    ) -> Result<crate::identifier::IdentifierSet> {
        crate::identifier::from_pmid(
            &self.http,
            &self.cfg.ncbi_base_url,
            self.cfg.api_key.as_deref(),
            pmid,
        )
        .await
    }

    /// Extract database identifiers from a DOI (resolves to PMID then PMC).
    pub async fn identifiers_from_doi(
        &self,
        doi: &str,
    ) -> Result<crate::identifier::IdentifierSet> {
        crate::identifier::from_doi(
            &self.http,
            &self.cfg.ncbi_base_url,
            self.cfg.api_key.as_deref(),
            doi,
        )
        .await
    }

    /// Extract database identifiers from a PMC ID.
    pub async fn identifiers_from_pmc(
        &self,
        pmc: &str,
    ) -> Result<crate::identifier::IdentifierSet> {
        crate::identifier::from_pmc(
            &self.http,
            &self.cfg.ncbi_base_url,
            self.cfg.api_key.as_deref(),
            pmc,
        )
        .await
    }

    /// Download a GEO Series Matrix `.txt.gz` for a GSE accession.
    /// Returns the gzipped bytes; use `geo::matrix::parse_matrix_gz` to decode.
    pub async fn geo_matrix_download(&self, gse: &str) -> Result<Vec<u8>> {
        let url = crate::geo::matrix::matrix_url(gse)?;
        let raw = reqwest::Client::builder()
            .timeout(self.cfg.timeout)
            .user_agent(format!("sradb-rs/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("reqwest client build");
        let resp = raw
            .get(&url)
            .send()
            .await
            .map_err(|source| crate::error::SradbError::Http {
                endpoint: "geo_matrix",
                source,
            })?;
        if !resp.status().is_success() {
            return Err(crate::error::SradbError::Download {
                url,
                reason: format!("status {}", resp.status()),
            });
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|source| crate::error::SradbError::Http {
                endpoint: "geo_matrix",
                source,
            })?;
        Ok(bytes.to_vec())
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
