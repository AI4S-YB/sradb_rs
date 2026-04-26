//! Search the NCBI SRA database via esearch + esummary.
//!
//! Supports the same filter set as pysradb's `SraSearch`. Builds an Entrez
//! query term with field qualifiers, then runs the metadata orchestrator on
//! the resulting accession set.

use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::metadata;
use crate::model::MetadataRow;
use crate::ncbi::{esearch, esummary};
use crate::parse;

/// Search filters. All fields are optional; an empty query (no filters and no
/// free-text query) returns an error.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Free-text term (no field qualifier).
    pub query: Option<String>,
    /// Organism scientific name, e.g. `"Homo sapiens"`. Maps to `[ORGN]`.
    pub organism: Option<String>,
    /// Library strategy, e.g. `"RNA-Seq"`. Maps to `[STRA]`.
    pub strategy: Option<String>,
    /// Library source, e.g. `"TRANSCRIPTOMIC"`. Maps to `[SRC]`.
    pub source: Option<String>,
    /// Library selection, e.g. `"cDNA"`. Maps to `[SEL]`.
    pub selection: Option<String>,
    /// Library layout, `"SINGLE"` or `"PAIRED"`. Maps to `[LAY]`.
    pub layout: Option<String>,
    /// Platform, e.g. `"ILLUMINA"`. Maps to `[PLAT]`.
    pub platform: Option<String>,
    /// Max results to return (default 20, NCBI hard cap 500 per page).
    pub max: u32,
}

impl SearchQuery {
    #[must_use]
    pub fn new() -> Self {
        Self {
            max: 20,
            ..Self::default()
        }
    }
}

/// Build an Entrez query term from a `SearchQuery`. Returns `None` if the query
/// is empty (no filters, no free text).
#[must_use]
pub fn build_term(q: &SearchQuery) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(t) = &q.query {
        let t = t.trim();
        if !t.is_empty() {
            parts.push(quote_if_needed(t));
        }
    }
    push_qualifier(&mut parts, q.organism.as_deref(), "ORGN");
    push_qualifier(&mut parts, q.strategy.as_deref(), "STRA");
    push_qualifier(&mut parts, q.source.as_deref(), "SRC");
    push_qualifier(&mut parts, q.selection.as_deref(), "SEL");
    push_qualifier(&mut parts, q.layout.as_deref(), "LAY");
    push_qualifier(&mut parts, q.platform.as_deref(), "PLAT");

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" AND "))
    }
}

fn push_qualifier(parts: &mut Vec<String>, value: Option<&str>, qualifier: &str) {
    if let Some(v) = value {
        let v = v.trim();
        if !v.is_empty() {
            parts.push(format!("{}[{qualifier}]", quote_if_needed(v)));
        }
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.contains(char::is_whitespace) || s.contains('-') {
        format!("\"{s}\"")
    } else {
        s.to_owned()
    }
}

/// Run a search end-to-end: esearch (with constructed term) → esummary →
/// metadata rows. Returns up to `query.max` results.
pub async fn search(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    query: &SearchQuery,
) -> Result<Vec<MetadataRow>> {
    let term = build_term(query).ok_or_else(|| SradbError::Parse {
        endpoint: "search",
        message: "empty search query (no filters and no free text)".into(),
    })?;
    let max = if query.max == 0 {
        20
    } else {
        query.max.min(500)
    };

    let result = esearch::esearch(http, ncbi_base_url, "sra", &term, api_key, max).await?;
    if result.count == 0 {
        return Ok(Vec::new());
    }
    if result.webenv.is_empty() || result.query_key.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "search/esearch",
            message: format!("count={} but missing webenv/query_key", result.count),
        });
    }

    let body = esummary::esummary_with_history(
        http,
        ncbi_base_url,
        "sra",
        &result.webenv,
        &result.query_key,
        0,
        max,
        api_key,
    )
    .await?;
    let docs = parse::esummary::parse(&body)?;

    let mut rows: Vec<MetadataRow> = Vec::new();
    for d in docs {
        rows.extend(metadata::assemble_default_rows(d)?);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_yields_none() {
        assert!(build_term(&SearchQuery::new()).is_none());
    }

    #[test]
    fn single_qualifier() {
        let q = SearchQuery {
            organism: Some("Homo sapiens".into()),
            ..SearchQuery::new()
        };
        assert_eq!(build_term(&q).as_deref(), Some(r#""Homo sapiens"[ORGN]"#));
    }

    #[test]
    fn multiple_qualifiers_joined_by_and() {
        let q = SearchQuery {
            organism: Some("Homo sapiens".into()),
            strategy: Some("RNA-Seq".into()),
            platform: Some("ILLUMINA".into()),
            ..SearchQuery::new()
        };
        let term = build_term(&q).unwrap();
        assert!(term.contains(r#""Homo sapiens"[ORGN]"#));
        assert!(term.contains(r#""RNA-Seq"[STRA]"#));
        assert!(term.contains("ILLUMINA[PLAT]"));
        assert_eq!(term.matches(" AND ").count(), 2);
    }

    #[test]
    fn free_text_only() {
        let q = SearchQuery {
            query: Some("ARID1A breast cancer".into()),
            ..SearchQuery::new()
        };
        assert_eq!(build_term(&q).as_deref(), Some(r#""ARID1A breast cancer""#));
    }

    #[test]
    fn quote_if_needed_skips_unicode_safe_words() {
        assert_eq!(quote_if_needed("ILLUMINA"), "ILLUMINA");
        assert_eq!(quote_if_needed("Homo sapiens"), "\"Homo sapiens\"");
        assert_eq!(quote_if_needed("RNA-Seq"), "\"RNA-Seq\"");
    }

    #[test]
    fn empty_strings_are_skipped() {
        let q = SearchQuery {
            query: Some("   ".into()),
            organism: Some(String::new()),
            ..SearchQuery::new()
        };
        assert!(build_term(&q).is_none());
    }
}
