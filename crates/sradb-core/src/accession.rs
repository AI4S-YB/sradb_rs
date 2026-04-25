//! Typed accession identifiers used across the sradb-core API.
//!
//! Replaces pysradb's stringly-typed accession handling. Parsing is regex-based
//! and case-sensitive: NCBI/EBI accessions are upper-case by convention.

use std::fmt;
use std::str::FromStr;

use once_cell::sync::Lazy;
use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessionKind {
    Srp,
    Srx,
    Srs,
    Srr,
    Gse,
    Gsm,
    BioProject,
    Pmid,
    Doi,
    Pmc,
}

impl AccessionKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Srp => "SRP",
            Self::Srx => "SRX",
            Self::Srs => "SRS",
            Self::Srr => "SRR",
            Self::Gse => "GSE",
            Self::Gsm => "GSM",
            Self::BioProject => "BioProject",
            Self::Pmid => "PMID",
            Self::Doi => "DOI",
            Self::Pmc => "PMC",
        }
    }
}

impl fmt::Display for AccessionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accession {
    pub kind: AccessionKind,
    pub raw: String,
}

impl Accession {
    #[must_use]
    pub fn new(kind: AccessionKind, raw: impl Into<String>) -> Self {
        Self { kind, raw: raw.into() }
    }
}

impl fmt::Display for Accession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid accession `{input}`: {reason}")]
pub struct ParseAccessionError {
    pub input: String,
    pub reason: String,
}

impl ParseAccessionError {
    fn new(input: &str, reason: impl Into<String>) -> Self {
        Self { input: input.to_owned(), reason: reason.into() }
    }
}

// Order matters: more specific patterns (PMC, BioProject) before generic.
static PATTERNS: Lazy<Vec<(AccessionKind, Regex)>> = Lazy::new(|| {
    vec![
        (AccessionKind::Pmc, Regex::new(r"^PMC\d+$").unwrap()),
        (AccessionKind::BioProject, Regex::new(r"^PRJ[A-Z]{2}\d+$").unwrap()),
        (AccessionKind::Srp, Regex::new(r"^(?:SRP|ERP|DRP)\d{4,}$").unwrap()),
        (AccessionKind::Srx, Regex::new(r"^(?:SRX|ERX|DRX)\d{4,}$").unwrap()),
        (AccessionKind::Srs, Regex::new(r"^(?:SRS|ERS|DRS)\d{4,}$").unwrap()),
        (AccessionKind::Srr, Regex::new(r"^(?:SRR|ERR|DRR)\d{4,}$").unwrap()),
        (AccessionKind::Gse, Regex::new(r"^GSE\d+$").unwrap()),
        (AccessionKind::Gsm, Regex::new(r"^GSM\d+$").unwrap()),
        (AccessionKind::Pmid, Regex::new(r"^\d{1,9}$").unwrap()),
    ]
});

// DOI is matched separately (loose RFC-3987-ish; doesn't fit the prefix pattern).
static DOI_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^10\.\d{4,9}/[\-._;()/:A-Za-z0-9]+$").unwrap());

impl FromStr for Accession {
    type Err = ParseAccessionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ParseAccessionError::new(s, "empty input"));
        }
        if DOI_RE.is_match(trimmed) {
            return Ok(Self::new(AccessionKind::Doi, trimmed));
        }
        for (kind, re) in PATTERNS.iter() {
            if re.is_match(trimmed) {
                return Ok(Self::new(*kind, trimmed));
            }
        }
        Err(ParseAccessionError::new(s, "no recognized accession pattern"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_each_kind() {
        let cases = [
            ("SRP000941", AccessionKind::Srp),
            ("SRX1800476", AccessionKind::Srx),
            ("SRS1467643", AccessionKind::Srs),
            ("SRR3587912", AccessionKind::Srr),
            ("ERR3587912", AccessionKind::Srr),
            ("DRR0123456", AccessionKind::Srr),
            ("GSE56924", AccessionKind::Gse),
            ("GSM1371490", AccessionKind::Gsm),
            ("PRJNA257197", AccessionKind::BioProject),
            ("PMC10802650", AccessionKind::Pmc),
            ("39528918", AccessionKind::Pmid),
            ("10.12688/f1000research.18676.1", AccessionKind::Doi),
        ];
        for (input, expected_kind) in cases {
            let acc: Accession = input.parse().unwrap_or_else(|e| panic!("{input}: {e}"));
            assert_eq!(acc.kind, expected_kind, "for input {input}");
            assert_eq!(acc.raw, input);
        }
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "  ", "abc", "srp123", "SRP", "PRJ123", "10.x/y"] {
            assert!(bad.parse::<Accession>().is_err(), "should reject {bad:?}");
        }
    }

    #[test]
    fn trims_whitespace() {
        let acc: Accession = "  SRP000941 \n".parse().unwrap();
        assert_eq!(acc.kind, AccessionKind::Srp);
        assert_eq!(acc.raw, "SRP000941");
    }

    #[test]
    fn display_round_trips() {
        let acc = Accession::new(AccessionKind::Srp, "SRP000941");
        assert_eq!(acc.to_string(), "SRP000941");
    }
}
