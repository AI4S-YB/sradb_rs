//! Extract database identifiers (GSE, GSM, SRP, PRJNA) from PMC fulltext.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Identifiers found in a PubMed / PMC article.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IdentifierSet {
    /// Source PubMed ID.
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
/// (pmid, pmc_id, doi) are not modified — set them externally.
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
        assert_eq!(set.gse_ids, vec!["GSE100".to_string(), "GSE999".to_string()]);
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
