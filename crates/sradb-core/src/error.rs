//! Library-level error type. CLI wraps these with anyhow context.

use std::path::PathBuf;

use crate::accession::AccessionKind;

#[derive(Debug, thiserror::Error)]
pub enum SradbError {
    #[error("invalid accession `{input}`: {reason}")]
    InvalidAccession { input: String, reason: String },

    #[error("accession not found: {0}")]
    NotFound(String),

    #[error("conversion not supported: {from:?} -> {to:?}")]
    UnsupportedConversion {
        from: AccessionKind,
        to: AccessionKind,
    },

    #[error("HTTP error from {endpoint}: {source}")]
    Http {
        endpoint: &'static str,
        #[source]
        source: reqwest::Error,
    },

    #[error("HTTP middleware error from {endpoint}: {source}")]
    HttpMiddleware {
        endpoint: &'static str,
        #[source]
        source: reqwest_middleware::Error,
    },

    #[error("rate limited by {service} after {retries} retries")]
    RateLimited { service: &'static str, retries: u32 },

    #[error("response parse error at {endpoint}: {message}")]
    Parse {
        endpoint: &'static str,
        message: String,
    },

    #[error("XML parse error in {context}: {source}")]
    Xml {
        context: &'static str,
        #[source]
        source: quick_xml::Error,
    },

    #[error("CSV parse error in {context}: {source}")]
    Csv {
        context: &'static str,
        #[source]
        source: csv::Error,
    },

    #[error("JSON parse error in {context}: {source}")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("enrichment failed: {message}")]
    Enrichment {
        message: String,
        #[source]
        source: Option<reqwest::Error>,
    },

    #[error("download failed for {url}: {reason}")]
    Download { url: String, reason: String },

    #[error("checksum mismatch for {path}: expected {expected}, got {got}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        got: String,
    },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, SradbError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_context() {
        let err = SradbError::Parse {
            endpoint: "esummary",
            message: "missing field foo".into(),
        };
        let s = format!("{err}");
        assert!(s.contains("esummary"), "got: {s}");
        assert!(s.contains("missing field foo"), "got: {s}");
    }

    #[test]
    fn io_from_conversion() {
        let io: std::io::Error = std::io::ErrorKind::NotFound.into();
        let e: SradbError = io.into();
        assert!(matches!(e, SradbError::Io(_)));
    }

    #[test]
    fn unsupported_conversion_lists_kinds() {
        let e = SradbError::UnsupportedConversion {
            from: AccessionKind::Srp,
            to: AccessionKind::Pmid,
        };
        let s = format!("{e}");
        assert!(s.contains("Srp"));
        assert!(s.contains("Pmid"));
    }
}
