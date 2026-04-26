//! Accession conversion engine.
//!
//! Replaces pysradb's 25+ separate conversion methods with a single
//! `(from_kind, to_kind) → Strategy` lookup. Two strategies handle every case:
//! - `ProjectFromMetadata`: call the metadata orchestrator, project a field per row.
//! - `GdsLookup`: call db=gds esearch+esummary, project a field from the JSON.
//!
//! Slice 4 implements both strategies. Chained conversions (e.g. GSE→SRX via
//! GSE→SRP→SRX) are handled by `Strategy::Chain`.

use std::collections::HashSet;

use crate::accession::{Accession, AccessionKind};
use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::metadata;
use crate::model::{MetadataOpts, MetadataRow};
use crate::ncbi::gds as ncbi_gds;
use crate::parse;

/// One field projector for the `ProjectFromMetadata` strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjField {
    StudyAccession,
    ExperimentAccession,
    RunAccession,
    SampleAccession,
    /// GSM accession parsed out of the experiment title (`"GSM3526037: ..."`).
    GeoExperimentFromTitle,
}

/// One field projector for the `GdsLookup` strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GdsField {
    /// `record.accession` for a record where `entrytype == "GSE"`.
    GseAccession,
    /// First `extrelations` entry whose `target_object` starts with `SRP`/`ERP`/`DRP`.
    SrpFromExtrelations,
    /// All child `samples[].accession` values.
    GsmsFromSamples,
    /// For a GSM record: the parent GSE — derived from `extrelations` or by
    /// chaining GSM→SRP→GSE. Slice 4 prefers the chain; documenting the field
    /// for completeness.
    GseFromGsmExtrelations,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// Diagonal: return the input unchanged.
    Identity,
    ProjectFromMetadata(ProjField),
    GdsLookup(GdsField),
    /// Chain: convert through an intermediate kind first.
    Chain { via: AccessionKind, second: ChainStep },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStep {
    /// After the chain's first leg yields one or more accessions, run convert
    /// again from `via` to the final target.
    Next,
}

/// Look up the strategy for converting `from` → `to`. Returns `None` for
/// unsupported pairs.
#[must_use]
pub fn strategy_for(from: AccessionKind, to: AccessionKind) -> Option<Strategy> {
    use AccessionKind::*;
    if from == to {
        return Some(Strategy::Identity);
    }
    let s = match (from, to) {
        // SRA family ↔ SRA family + GSM via metadata projection
        (Srp, Srx) | (Srr, Srx) | (Srs, Srx) | (Gsm, Srx) => Strategy::ProjectFromMetadata(ProjField::ExperimentAccession),
        (Srp, Srr) | (Srx, Srr) | (Gsm, Srr) => Strategy::ProjectFromMetadata(ProjField::RunAccession),
        (Srp, Srs) | (Srx, Srs) | (Srr, Srs) | (Gsm, Srs) => Strategy::ProjectFromMetadata(ProjField::SampleAccession),
        (Srx, Srp) | (Srr, Srp) | (Gsm, Srp) => Strategy::ProjectFromMetadata(ProjField::StudyAccession),
        (Srx, Gsm) | (Srr, Gsm) | (Srs, Gsm) => Strategy::ProjectFromMetadata(ProjField::GeoExperimentFromTitle),

        // GSE-related: db=gds path
        (Srp, Gse) => Strategy::GdsLookup(GdsField::GseAccession),
        (Gsm, Gse) => Strategy::Chain { via: Srp, second: ChainStep::Next }, // GSM→SRP→GSE
        (Gse, Srp) => Strategy::GdsLookup(GdsField::SrpFromExtrelations),
        (Gse, Gsm) => Strategy::GdsLookup(GdsField::GsmsFromSamples),

        // Chained conversions involving GSE on either side
        (Gse, Srx) | (Gse, Srr) | (Gse, Srs) => Strategy::Chain { via: Srp, second: ChainStep::Next }, // GSE→SRP→target
        (Srs, Srp) => Strategy::Chain { via: Srx, second: ChainStep::Next }, // SRS→SRX→SRP (pysradb skips Srs→Srp directly)

        _ => return None,
    };
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AccessionKind::*;

    #[test]
    fn diagonal_is_identity() {
        for k in [Srp, Srx, Srr, Srs, Gse, Gsm] {
            assert_eq!(strategy_for(k, k), Some(Strategy::Identity), "k={:?}", k);
        }
    }

    #[test]
    fn supported_pairs_have_strategies() {
        // Every cell with a check-mark in the conversion table.
        let pairs = [
            (Srp, Srx), (Srp, Srr), (Srp, Srs), (Srp, Gse),
            (Srx, Srp), (Srx, Srr), (Srx, Srs), (Srx, Gsm),
            (Srr, Srp), (Srr, Srx), (Srr, Srs), (Srr, Gsm),
            (Srs, Srx), (Srs, Gsm),
            (Gse, Srp), (Gse, Gsm),
            (Gsm, Srp), (Gsm, Srx), (Gsm, Srr), (Gsm, Srs), (Gsm, Gse),
        ];
        for (from, to) in pairs {
            assert!(strategy_for(from, to).is_some(), "missing strategy for {:?} → {:?}", from, to);
        }
    }

    #[test]
    fn unsupported_pairs_return_none() {
        assert!(strategy_for(Pmid, Srp).is_none());
        assert!(strategy_for(Srp, Pmid).is_none());
        assert!(strategy_for(Doi, Pmc).is_none());
    }
}
