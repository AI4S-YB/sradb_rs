//! Stub. Filled in Task 4.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessionKind { Srp, Srx, Srs, Srr, Gse, Gsm, BioProject, Pmid, Doi, Pmc }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Accession { pub kind: AccessionKind, pub raw: String }

#[derive(Debug, thiserror::Error)]
#[error("invalid accession `{input}`: {reason}")]
pub struct ParseAccessionError { pub input: String, pub reason: String }
