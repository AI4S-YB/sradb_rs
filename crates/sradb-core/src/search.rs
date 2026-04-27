//! Search backends for SRA, ENA, and GEO.
//!
//! - `db sra`: NCBI esearch + esummary on `db=sra` → `Vec<MetadataRow>`.
//! - `db ena`: ENA portal API (`/portal/api/search?result=read_run`, TSV) → `Vec<EnaSearchHit>`.
//! - `db geo`: NCBI esearch + esummary on `db=gds` → `Vec<GeoSearchHit>`.

use serde::Serialize;

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};
use crate::metadata;
use crate::model::MetadataRow;
use crate::ncbi::{esearch, esummary, gds};
use crate::parse;

/// Search filters. All fields are optional; an empty query (no filters and no
/// free-text query) returns an error.
#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    /// Free-text term (no field qualifier).
    pub query: Option<String>,
    /// Organism scientific name, e.g. `"Homo sapiens"`. Maps to `[ORGN]` (SRA/GEO)
    /// or `tax_name="..."` (ENA).
    pub organism: Option<String>,
    /// Library strategy, e.g. `"RNA-Seq"`. SRA: `[STRA]`, ENA: `library_strategy`.
    pub strategy: Option<String>,
    /// Library source, e.g. `"TRANSCRIPTOMIC"`. SRA: `[SRC]`, ENA: `library_source`.
    pub source: Option<String>,
    /// Library selection, e.g. `"cDNA"`. SRA: `[SEL]`, ENA: `library_selection`.
    pub selection: Option<String>,
    /// Library layout, `"SINGLE"` or `"PAIRED"`. SRA: `[LAY]`, ENA: `library_layout`.
    pub layout: Option<String>,
    /// Platform, e.g. `"ILLUMINA"`. SRA: `[PLAT]`, ENA: `instrument_platform`.
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
/// is empty (no filters, no free text). Used by the SRA and GEO backends.
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

/// Run an SRA search end-to-end: esearch (with constructed term) → esummary →
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
    let max = clamp_max(query.max);

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

/// One row of an ENA portal `read_run` search result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EnaSearchHit {
    pub run_accession: String,
    pub experiment_accession: String,
    pub sample_accession: String,
    pub study_accession: String,
    pub scientific_name: Option<String>,
    pub library_strategy: Option<String>,
    pub library_source: Option<String>,
    pub library_selection: Option<String>,
    pub library_layout: Option<String>,
    pub instrument_platform: Option<String>,
    pub instrument_model: Option<String>,
    pub read_count: Option<u64>,
    pub base_count: Option<u64>,
    pub study_title: Option<String>,
}

/// ENA portal API field list we request. Order is mirrored when emitting TSV.
pub const ENA_SEARCH_FIELDS: &[&str] = &[
    "run_accession",
    "experiment_accession",
    "sample_accession",
    "study_accession",
    "scientific_name",
    "library_strategy",
    "library_source",
    "library_selection",
    "library_layout",
    "instrument_platform",
    "instrument_model",
    "read_count",
    "base_count",
    "study_title",
];

/// Build an ENA portal query expression. Returns `None` when no filters are set.
#[must_use]
pub fn build_ena_query(q: &SearchQuery) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    push_ena_eq(&mut parts, "tax_name", q.organism.as_deref());
    push_ena_eq(&mut parts, "library_strategy", q.strategy.as_deref());
    push_ena_eq(&mut parts, "library_source", q.source.as_deref());
    push_ena_eq(&mut parts, "library_selection", q.selection.as_deref());
    push_ena_eq(&mut parts, "library_layout", q.layout.as_deref());
    push_ena_eq(&mut parts, "instrument_platform", q.platform.as_deref());

    if let Some(t) = q.query.as_deref() {
        let t = t.trim();
        if !t.is_empty() {
            // Free-text → LIKE match against study_title. ENA portal's `query`
            // grammar has no cross-field full-text operator; this is the
            // closest pragmatic mapping.
            let escaped = t.replace('"', "\\\"");
            parts.push(format!("study_title=\"*{escaped}*\""));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" AND "))
    }
}

fn push_ena_eq(parts: &mut Vec<String>, field: &str, value: Option<&str>) {
    if let Some(v) = value {
        let v = v.trim();
        if !v.is_empty() {
            let escaped = v.replace('"', "\\\"");
            parts.push(format!("{field}=\"{escaped}\""));
        }
    }
}

/// Run an ENA portal search end-to-end. Returns up to `query.max` rows.
pub async fn search_ena(
    http: &HttpClient,
    ena_base_url: &str,
    query: &SearchQuery,
) -> Result<Vec<EnaSearchHit>> {
    let expr = build_ena_query(query).ok_or_else(|| SradbError::Parse {
        endpoint: "search/ena",
        message: "empty search query (no filters and no free text)".into(),
    })?;
    let max = clamp_max(query.max);
    let max_str = max.to_string();
    let fields = ENA_SEARCH_FIELDS.join(",");

    let url = format!("{ena_base_url}/portal/api/search");
    let body = http
        .get_text(
            "ena_search",
            Service::Ena,
            &url,
            &[
                ("result", "read_run"),
                ("query", &expr),
                ("fields", &fields),
                ("format", "tsv"),
                ("limit", &max_str),
            ],
        )
        .await?;
    parse_ena_search_tsv(&body)
}

/// Parse the TSV body returned by `/portal/api/search`. The first line is the
/// header (column names); subsequent lines are tab-separated values.
pub fn parse_ena_search_tsv(body: &str) -> Result<Vec<EnaSearchHit>> {
    let mut lines = body.lines();
    let header = match lines.next() {
        Some(h) if !h.trim().is_empty() => h,
        _ => return Ok(Vec::new()),
    };
    let cols: Vec<&str> = header.split('\t').collect();

    let mut hits = Vec::new();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let vals: Vec<&str> = line.split('\t').collect();
        let mut hit = EnaSearchHit::default();
        for (i, col) in cols.iter().enumerate() {
            let v = vals.get(i).copied().unwrap_or("").trim();
            assign_ena_field(&mut hit, col, v);
        }
        hits.push(hit);
    }
    Ok(hits)
}

fn assign_ena_field(hit: &mut EnaSearchHit, col: &str, v: &str) {
    let opt = if v.is_empty() {
        None
    } else {
        Some(v.to_owned())
    };
    match col {
        "run_accession" => v.clone_into(&mut hit.run_accession),
        "experiment_accession" => v.clone_into(&mut hit.experiment_accession),
        "sample_accession" => v.clone_into(&mut hit.sample_accession),
        "study_accession" => v.clone_into(&mut hit.study_accession),
        "scientific_name" => hit.scientific_name = opt,
        "library_strategy" => hit.library_strategy = opt,
        "library_source" => hit.library_source = opt,
        "library_selection" => hit.library_selection = opt,
        "library_layout" => hit.library_layout = opt,
        "instrument_platform" => hit.instrument_platform = opt,
        "instrument_model" => hit.instrument_model = opt,
        "read_count" => hit.read_count = v.parse().ok(),
        "base_count" => hit.base_count = v.parse().ok(),
        "study_title" => hit.study_title = opt,
        _ => {}
    }
}

/// One row of a GEO db=gds search result (a GSE/GSM/GPL record).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct GeoSearchHit {
    pub accession: String,
    pub entry_type: String,
    pub n_samples: Option<u32>,
    /// SRA cross-link from the GDS `extrelations` block, when present
    /// (typically an SRP for a GSE, an SRX for a GSM).
    pub sra_accession: Option<String>,
}

/// Run a GEO db=gds search end-to-end. Builds an Entrez term identical to the
/// SRA path (so `[ORGN]`/`[STRA]` still work), then esearch+esummary on
/// `db=gds` and projects the JSON into `GeoSearchHit`s.
pub async fn search_geo(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    query: &SearchQuery,
) -> Result<Vec<GeoSearchHit>> {
    let term = build_term(query).ok_or_else(|| SradbError::Parse {
        endpoint: "search/geo",
        message: "empty search query (no filters and no free text)".into(),
    })?;
    let max = clamp_max(query.max);

    let result = esearch::esearch(http, ncbi_base_url, "gds", &term, api_key, max).await?;
    if result.count == 0 {
        return Ok(Vec::new());
    }
    if result.ids.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "search/geo/esearch",
            message: format!("count={} but no UIDs returned", result.count),
        });
    }

    let body = gds::gds_esummary_by_uids(http, ncbi_base_url, &result.ids, api_key).await?;
    let records = parse::gds_esummary::parse(&body)?;

    let hits = records
        .into_iter()
        .map(|r| GeoSearchHit {
            accession: r.accession,
            entry_type: r.entry_type,
            n_samples: r.n_samples,
            sra_accession: r
                .extrelations
                .into_iter()
                .find(|x| x.relation_type == "SRA")
                .map(|x| x.target_object),
        })
        .collect();
    Ok(hits)
}

fn clamp_max(max: u32) -> u32 {
    if max == 0 {
        20
    } else {
        max.min(500)
    }
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

    #[test]
    fn ena_query_combines_filters_with_and() {
        let q = SearchQuery {
            organism: Some("Homo sapiens".into()),
            strategy: Some("RNA-Seq".into()),
            platform: Some("ILLUMINA".into()),
            ..SearchQuery::new()
        };
        let expr = build_ena_query(&q).unwrap();
        assert!(expr.contains("tax_name=\"Homo sapiens\""));
        assert!(expr.contains("library_strategy=\"RNA-Seq\""));
        assert!(expr.contains("instrument_platform=\"ILLUMINA\""));
        assert_eq!(expr.matches(" AND ").count(), 2);
    }

    #[test]
    fn ena_query_free_text_uses_like_on_title() {
        let q = SearchQuery {
            query: Some("breast cancer".into()),
            ..SearchQuery::new()
        };
        let expr = build_ena_query(&q).unwrap();
        assert_eq!(expr, r#"study_title="*breast cancer*""#);
    }

    #[test]
    fn ena_query_empty_returns_none() {
        assert!(build_ena_query(&SearchQuery::new()).is_none());
    }

    #[test]
    fn parse_ena_search_tsv_handles_header_and_blank_lines() {
        let body = "run_accession\tstudy_accession\tlibrary_strategy\tread_count\n\
                    SRR1\tSRP1\tRNA-Seq\t1234\n\
                    \n\
                    SRR2\tSRP2\t\t\n";
        let hits = parse_ena_search_tsv(body).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].run_accession, "SRR1");
        assert_eq!(hits[0].study_accession, "SRP1");
        assert_eq!(hits[0].library_strategy.as_deref(), Some("RNA-Seq"));
        assert_eq!(hits[0].read_count, Some(1234));
        assert_eq!(hits[1].run_accession, "SRR2");
        assert_eq!(hits[1].library_strategy, None);
        assert_eq!(hits[1].read_count, None);
    }

    #[test]
    fn parse_ena_search_tsv_empty_body_returns_empty() {
        assert!(parse_ena_search_tsv("").unwrap().is_empty());
    }
}
