//! Extract database identifiers (GSE, GSM, SRP, PRJNA) from PMC fulltext.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Identifiers found in a `PubMed` / PMC article.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IdentifierSet {
    /// Source `PubMed` ID.
    pub pmid: Option<u64>,
    /// Source PMC ID (with `PMC` prefix).
    pub pmc_id: Option<String>,
    /// Source DOI.
    pub doi: Option<String>,
    pub gse_ids: Vec<String>,
    pub gsm_ids: Vec<String>,
    pub srp_ids: Vec<String>,
    pub prjna_ids: Vec<String>,
}

/// Run all four regexes over the body and populate ID lists. Existing fields
/// (`pmid`, `pmc_id`, `doi`) are not modified — set them externally.
pub fn extract_into(body: &str, set: &mut IdentifierSet) {
    static GSE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bGSE\d{3,}\b").unwrap());
    static GSM_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\bGSM\d{3,}\b").unwrap());
    static SRP_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b[ESDR]?SRP\d{4,}\b|\b[EDS]RP\d{4,}\b").unwrap());
    static PRJ_RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\bPRJ[A-Z]{2}\d{4,}\b").unwrap());

    set.gse_ids = dedup_sorted(GSE_RE.find_iter(body).map(|m| m.as_str().to_owned()));
    set.gsm_ids = dedup_sorted(GSM_RE.find_iter(body).map(|m| m.as_str().to_owned()));
    set.srp_ids = dedup_sorted(SRP_RE.find_iter(body).map(|m| m.as_str().to_owned()));
    set.prjna_ids = dedup_sorted(PRJ_RE.find_iter(body).map(|m| m.as_str().to_owned()));
}

fn dedup_sorted(items: impl Iterator<Item = String>) -> Vec<String> {
    let set: BTreeSet<String> = items.collect();
    set.into_iter().collect()
}

use crate::error::{Result, SradbError};
use crate::http::{HttpClient, Service};
use crate::ncbi::{elink, esearch};

/// `PubMed` → PMC → fulltext → identifiers.
pub async fn from_pmid(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    pmid: u64,
) -> Result<IdentifierSet> {
    let mut set = IdentifierSet {
        pmid: Some(pmid),
        ..IdentifierSet::default()
    };
    let pmc_ids = elink::pmid_to_pmc_ids(http, base_url, pmid, api_key).await?;
    if let Some(pmc_numeric) = pmc_ids.first() {
        set.pmc_id = Some(format!("PMC{pmc_numeric}"));
        let body = fetch_pmc_fulltext(http, base_url, pmc_numeric, api_key).await?;
        extract_into(&body, &mut set);
    }
    Ok(set)
}

/// DOI → PMID → PMC → identifiers.
pub async fn from_doi(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    doi: &str,
) -> Result<IdentifierSet> {
    let term = format!("{doi}[doi]");
    let result = esearch::esearch(http, base_url, "pubmed", &term, api_key, 1).await?;
    let pmid_str = result
        .ids
        .first()
        .ok_or_else(|| SradbError::NotFound(format!("DOI {doi} not in PubMed")))?;
    let pmid: u64 = pmid_str.parse().map_err(|_| SradbError::Parse {
        endpoint: "doi_to_pmid",
        message: format!("non-numeric PMID `{pmid_str}`"),
    })?;
    let mut set = from_pmid(http, base_url, api_key, pmid).await?;
    set.doi = Some(doi.to_owned());
    Ok(set)
}

/// PMC → fulltext → identifiers.
pub async fn from_pmc(
    http: &HttpClient,
    base_url: &str,
    api_key: Option<&str>,
    pmc: &str,
) -> Result<IdentifierSet> {
    let pmc_numeric = pmc.trim_start_matches("PMC");
    if pmc_numeric.is_empty() || !pmc_numeric.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SradbError::InvalidAccession {
            input: pmc.to_owned(),
            reason: "expected PMC<digits>".into(),
        });
    }
    let mut set = IdentifierSet {
        pmc_id: Some(format!("PMC{pmc_numeric}")),
        ..IdentifierSet::default()
    };
    let body = fetch_pmc_fulltext(http, base_url, pmc_numeric, api_key).await?;
    extract_into(&body, &mut set);
    Ok(set)
}

async fn fetch_pmc_fulltext(
    http: &HttpClient,
    base_url: &str,
    pmc_numeric: &str,
    api_key: Option<&str>,
) -> Result<String> {
    let url = format!("{base_url}/efetch.fcgi");
    let mut q: Vec<(&str, &str)> = vec![
        ("db", "pmc"),
        ("id", pmc_numeric),
        ("rettype", "full"),
        ("retmode", "xml"),
    ];
    if let Some(k) = api_key {
        q.push(("api_key", k));
    }
    http.get_text("efetch_pmc", Service::Ncbi, &url, &q).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_gse_and_srp() {
        let body = "Data deposited at GSE253406 and SRP484103 are publicly available.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert_eq!(set.gse_ids, vec!["GSE253406".to_string()]);
        assert_eq!(set.srp_ids, vec!["SRP484103".to_string()]);
    }

    #[test]
    fn extracts_prjna() {
        let body = "BioProject PRJNA1058002 contains the raw reads.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert_eq!(set.prjna_ids, vec!["PRJNA1058002".to_string()]);
    }

    #[test]
    fn dedup_and_sort() {
        let body = "We used GSE100 and GSE999 and GSE100 again, plus GSE999.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert_eq!(
            set.gse_ids,
            vec!["GSE100".to_string(), "GSE999".to_string()]
        );
    }

    #[test]
    fn empty_body_yields_empty_lists() {
        let mut set = IdentifierSet::default();
        extract_into("", &mut set);
        assert!(set.gse_ids.is_empty());
        assert!(set.srp_ids.is_empty());
        assert!(set.prjna_ids.is_empty());
        assert!(set.gsm_ids.is_empty());
    }

    #[test]
    fn skips_partial_matches() {
        let body = "GSE12 is too short, GSE123 is fine. SRP12 too short, SRP1234 fine.";
        let mut set = IdentifierSet::default();
        extract_into(body, &mut set);
        assert!(!set.gse_ids.contains(&"GSE12".to_string()));
        assert!(set.gse_ids.contains(&"GSE123".to_string()));
        assert!(set.srp_ids.contains(&"SRP1234".to_string()));
    }
}
