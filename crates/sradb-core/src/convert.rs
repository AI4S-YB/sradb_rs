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

const GSM_TITLE_RE: &str = r"GSM\d{3,}";

fn project_metadata_row(row: &MetadataRow, field: ProjField) -> Option<String> {
    match field {
        ProjField::StudyAccession => non_empty(row.study.accession.clone()),
        ProjField::ExperimentAccession => non_empty(row.experiment.accession.clone()),
        ProjField::RunAccession => non_empty(row.run.accession.clone()),
        ProjField::SampleAccession => non_empty(row.sample.accession.clone()),
        ProjField::GeoExperimentFromTitle => row
            .experiment
            .title
            .as_deref()
            .and_then(extract_gsm),
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() { None } else { Some(s) }
}

fn extract_gsm(title: &str) -> Option<String> {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(GSM_TITLE_RE).unwrap());
    RE.find(title).map(|m| m.as_str().to_owned())
}

/// Execute `ProjectFromMetadata`: call `metadata::fetch_metadata` and project the field.
pub async fn execute_project_from_metadata(
    http: &HttpClient,
    ncbi_base_url: &str,
    ena_base_url: &str,
    api_key: Option<&str>,
    input: &Accession,
    field: ProjField,
) -> Result<Vec<String>> {
    let opts = MetadataOpts { detailed: false, enrich: false, page_size: 500 };
    let rows = metadata::fetch_metadata(http, ncbi_base_url, ena_base_url, api_key, &input.raw, &opts).await?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for row in &rows {
        if let Some(v) = project_metadata_row(row, field) {
            if seen.insert(v.clone()) {
                out.push(v);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod project_tests {
    use super::*;

    fn fixture_row() -> MetadataRow {
        use crate::model::{Experiment, Library, Platform, Run, RunUrls, Sample, Study};
        MetadataRow {
            run: Run {
                accession: "SRR8361601".into(),
                experiment_accession: "SRX5172107".into(),
                sample_accession: "SRS4179725".into(),
                study_accession: "SRP174132".into(),
                ..Run::default()
            },
            experiment: Experiment {
                accession: "SRX5172107".into(),
                title: Some("GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq".into()),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                library: Library::default(),
                platform: Platform::default(),
                ..Experiment::default()
            },
            sample: Sample { accession: "SRS4179725".into(), ..Sample::default() },
            study: Study { accession: "SRP174132".into(), ..Study::default() },
            enrichment: None,
        }
    }

    #[test]
    fn project_each_field() {
        let row = fixture_row();
        assert_eq!(project_metadata_row(&row, ProjField::StudyAccession).as_deref(), Some("SRP174132"));
        assert_eq!(project_metadata_row(&row, ProjField::ExperimentAccession).as_deref(), Some("SRX5172107"));
        assert_eq!(project_metadata_row(&row, ProjField::RunAccession).as_deref(), Some("SRR8361601"));
        assert_eq!(project_metadata_row(&row, ProjField::SampleAccession).as_deref(), Some("SRS4179725"));
        assert_eq!(project_metadata_row(&row, ProjField::GeoExperimentFromTitle).as_deref(), Some("GSM3526037"));
    }

    #[test]
    fn extract_gsm_misc() {
        assert_eq!(extract_gsm("GSM12345: bla"), Some("GSM12345".to_string()));
        assert_eq!(extract_gsm("RNA-Seq sample"), None);
        assert_eq!(extract_gsm("preamble GSM999 trailing"), Some("GSM999".to_string()));
    }
}
