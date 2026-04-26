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
    Chain {
        via: AccessionKind,
        second: ChainStep,
    },
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
    use AccessionKind::{Gse, Gsm, Srp, Srr, Srs, Srx};
    if from == to {
        return Some(Strategy::Identity);
    }
    let s = match (from, to) {
        // SRA family ↔ SRA family + GSM via metadata projection
        (Srp | Srr | Srs | Gsm, Srx) => {
            Strategy::ProjectFromMetadata(ProjField::ExperimentAccession)
        }
        (Srp | Srx | Gsm, Srr) => Strategy::ProjectFromMetadata(ProjField::RunAccession),
        (Srp | Srx | Srr | Gsm, Srs) => Strategy::ProjectFromMetadata(ProjField::SampleAccession),
        (Srx | Srr | Gsm, Srp) => Strategy::ProjectFromMetadata(ProjField::StudyAccession),
        (Srx | Srr | Srs, Gsm) => Strategy::ProjectFromMetadata(ProjField::GeoExperimentFromTitle),

        // GSE-related: db=gds path
        (Srp, Gse) => Strategy::GdsLookup(GdsField::GseAccession),
        (Gse, Srp) => Strategy::GdsLookup(GdsField::SrpFromExtrelations),
        (Gse, Gsm) => Strategy::GdsLookup(GdsField::GsmsFromSamples),

        // Chained conversions: 2 legs through an intermediate kind.
        // GSM→GSE goes via SRP (extrelations of GSM record points to SRX, not GSE).
        // GSE→{SRX,SRR,SRS} goes via SRP.
        (Gsm, Gse) | (Gse, Srx | Srr | Srs) => Strategy::Chain {
            via: Srp,
            second: ChainStep::Next,
        },
        // SRS→SRP via SRX (pysradb skips SRS→SRP directly).
        (Srs, Srp) => Strategy::Chain {
            via: Srx,
            second: ChainStep::Next,
        },

        _ => return None,
    };
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use AccessionKind::{Doi, Gse, Gsm, Pmc, Pmid, Srp, Srr, Srs, Srx};

    #[test]
    fn diagonal_is_identity() {
        for k in [Srp, Srx, Srr, Srs, Gse, Gsm] {
            assert_eq!(strategy_for(k, k), Some(Strategy::Identity), "k={k:?}");
        }
    }

    #[test]
    fn supported_pairs_have_strategies() {
        // Every cell with a check-mark in the conversion table.
        let pairs = [
            (Srp, Srx),
            (Srp, Srr),
            (Srp, Srs),
            (Srp, Gse),
            (Srx, Srp),
            (Srx, Srr),
            (Srx, Srs),
            (Srx, Gsm),
            (Srr, Srp),
            (Srr, Srx),
            (Srr, Srs),
            (Srr, Gsm),
            (Srs, Srx),
            (Srs, Gsm),
            (Gse, Srp),
            (Gse, Gsm),
            (Gsm, Srp),
            (Gsm, Srx),
            (Gsm, Srr),
            (Gsm, Srs),
            (Gsm, Gse),
        ];
        for (from, to) in pairs {
            assert!(
                strategy_for(from, to).is_some(),
                "missing strategy for {from:?} → {to:?}"
            );
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
        ProjField::GeoExperimentFromTitle => row.experiment.title.as_deref().and_then(extract_gsm),
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
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
    let opts = MetadataOpts {
        detailed: false,
        enrich: false,
        page_size: 500,
    };
    let rows = metadata::fetch_metadata(
        http,
        ncbi_base_url,
        ena_base_url,
        api_key,
        &input.raw,
        &opts,
    )
    .await?;
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
        use crate::model::{Experiment, Library, Platform, Run, Sample, Study};
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
                title: Some(
                    "GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq".into(),
                ),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                library: Library::default(),
                platform: Platform::default(),
                ..Experiment::default()
            },
            sample: Sample {
                accession: "SRS4179725".into(),
                ..Sample::default()
            },
            study: Study {
                accession: "SRP174132".into(),
                ..Study::default()
            },
            enrichment: None,
        }
    }

    #[test]
    fn project_each_field() {
        let row = fixture_row();
        assert_eq!(
            project_metadata_row(&row, ProjField::StudyAccession).as_deref(),
            Some("SRP174132")
        );
        assert_eq!(
            project_metadata_row(&row, ProjField::ExperimentAccession).as_deref(),
            Some("SRX5172107")
        );
        assert_eq!(
            project_metadata_row(&row, ProjField::RunAccession).as_deref(),
            Some("SRR8361601")
        );
        assert_eq!(
            project_metadata_row(&row, ProjField::SampleAccession).as_deref(),
            Some("SRS4179725")
        );
        assert_eq!(
            project_metadata_row(&row, ProjField::GeoExperimentFromTitle).as_deref(),
            Some("GSM3526037")
        );
    }

    #[test]
    fn extract_gsm_misc() {
        assert_eq!(extract_gsm("GSM12345: bla"), Some("GSM12345".to_string()));
        assert_eq!(extract_gsm("RNA-Seq sample"), None);
        assert_eq!(
            extract_gsm("preamble GSM999 trailing"),
            Some("GSM999".to_string())
        );
    }
}

/// Execute `GdsLookup`: db=gds esearch + esummary, project a field.
pub async fn execute_gds_lookup(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    input: &Accession,
    field: GdsField,
) -> Result<Vec<String>> {
    let uids = ncbi_gds::gds_esearch_uids(http, ncbi_base_url, &input.raw, api_key).await?;
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let body = ncbi_gds::gds_esummary_by_uids(http, ncbi_base_url, &uids, api_key).await?;
    let records = parse::gds_esummary::parse(&body)?;

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for record in &records {
        match field {
            GdsField::GseAccession => {
                if record.entry_type == "GSE"
                    && !record.accession.is_empty()
                    && seen.insert(record.accession.clone())
                {
                    out.push(record.accession.clone());
                }
            }
            GdsField::SrpFromExtrelations => {
                for rel in &record.extrelations {
                    if (rel.target_object.starts_with("SRP")
                        || rel.target_object.starts_with("ERP")
                        || rel.target_object.starts_with("DRP"))
                        && seen.insert(rel.target_object.clone())
                    {
                        out.push(rel.target_object.clone());
                    }
                }
            }
            GdsField::GsmsFromSamples => {
                for s in &record.samples {
                    if !s.accession.is_empty() && seen.insert(s.accession.clone()) {
                        out.push(s.accession.clone());
                    }
                }
            }
            GdsField::GseFromGsmExtrelations => {
                // For GSM records, extrelations typically points to SRA (often SRX), not GSE.
                // Slice 4 prefers the chain GSM→SRP→GSE; this branch is a no-op.
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod gds_executor_tests {
    use super::*;
    use crate::parse::gds_esummary::{GdsExtRelation, GdsRecord, GdsSample};

    fn fake_gse_record() -> GdsRecord {
        GdsRecord {
            uid: "200056924".into(),
            accession: "GSE56924".into(),
            entry_type: "GSE".into(),
            n_samples: Some(2),
            samples: vec![
                GdsSample {
                    accession: "GSM1".into(),
                    title: "s1".into(),
                },
                GdsSample {
                    accession: "GSM2".into(),
                    title: "s2".into(),
                },
            ],
            extrelations: vec![GdsExtRelation {
                relation_type: "SRA".into(),
                target_object: "SRP041298".into(),
            }],
        }
    }

    fn project_field(record: &GdsRecord, field: GdsField) -> Vec<String> {
        // Mirror of execute_gds_lookup's per-record projection, factored out for unit testing.
        let mut out = Vec::new();
        match field {
            GdsField::GseAccession => {
                if record.entry_type == "GSE" && !record.accession.is_empty() {
                    out.push(record.accession.clone());
                }
            }
            GdsField::SrpFromExtrelations => {
                for rel in &record.extrelations {
                    if rel.target_object.starts_with("SRP") {
                        out.push(rel.target_object.clone());
                    }
                }
            }
            GdsField::GsmsFromSamples => {
                for s in &record.samples {
                    out.push(s.accession.clone());
                }
            }
            GdsField::GseFromGsmExtrelations => {}
        }
        out
    }

    #[test]
    fn project_gse_accession() {
        let r = fake_gse_record();
        assert_eq!(
            project_field(&r, GdsField::GseAccession),
            vec!["GSE56924".to_string()]
        );
    }

    #[test]
    fn project_srp_from_extrelations() {
        let r = fake_gse_record();
        assert_eq!(
            project_field(&r, GdsField::SrpFromExtrelations),
            vec!["SRP041298".to_string()]
        );
    }

    #[test]
    fn project_gsms_from_samples() {
        let r = fake_gse_record();
        assert_eq!(
            project_field(&r, GdsField::GsmsFromSamples),
            vec!["GSM1".to_string(), "GSM2".to_string()]
        );
    }
}

/// Top-level dispatch: convert one accession to a list of accessions of the target kind.
///
/// Dedupes the result. Returns an empty vec if the input maps to nothing.
/// Returns `Err(SradbError::UnsupportedConversion { ... })` for un-tabled pairs.
pub async fn convert_one(
    http: &HttpClient,
    ncbi_base_url: &str,
    ena_base_url: &str,
    api_key: Option<&str>,
    input: &Accession,
    to: AccessionKind,
) -> Result<Vec<Accession>> {
    let strategy = strategy_for(input.kind, to).ok_or(SradbError::UnsupportedConversion {
        from: input.kind,
        to,
    })?;
    convert_with_strategy(
        http,
        ncbi_base_url,
        ena_base_url,
        api_key,
        input,
        to,
        strategy,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
fn convert_with_strategy<'a>(
    http: &'a HttpClient,
    ncbi_base_url: &'a str,
    ena_base_url: &'a str,
    api_key: Option<&'a str>,
    input: &'a Accession,
    to: AccessionKind,
    strategy: Strategy,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<Accession>>> + Send + 'a>> {
    Box::pin(async move {
        match strategy {
            Strategy::Identity => Ok(vec![input.clone()]),
            Strategy::ProjectFromMetadata(field) => {
                let raws = execute_project_from_metadata(
                    http,
                    ncbi_base_url,
                    ena_base_url,
                    api_key,
                    input,
                    field,
                )
                .await?;
                Ok(raws
                    .into_iter()
                    .map(|raw| Accession { kind: to, raw })
                    .collect())
            }
            Strategy::GdsLookup(field) => {
                let raws = execute_gds_lookup(http, ncbi_base_url, api_key, input, field).await?;
                Ok(raws
                    .into_iter()
                    .map(|raw| Accession { kind: to, raw })
                    .collect())
            }
            Strategy::Chain {
                via,
                second: ChainStep::Next,
            } => {
                // First leg: input → via
                let first_strategy =
                    strategy_for(input.kind, via).ok_or(SradbError::UnsupportedConversion {
                        from: input.kind,
                        to: via,
                    })?;
                let mid = convert_with_strategy(
                    http,
                    ncbi_base_url,
                    ena_base_url,
                    api_key,
                    input,
                    via,
                    first_strategy,
                )
                .await?;
                // Second leg: each via → to
                let second_strategy = strategy_for(via, to)
                    .ok_or(SradbError::UnsupportedConversion { from: via, to })?;
                let mut seen = HashSet::new();
                let mut out: Vec<Accession> = Vec::new();
                for mid_acc in &mid {
                    let leg = convert_with_strategy(
                        http,
                        ncbi_base_url,
                        ena_base_url,
                        api_key,
                        mid_acc,
                        to,
                        second_strategy,
                    )
                    .await?;
                    for a in leg {
                        if seen.insert(a.raw.clone()) {
                            out.push(a);
                        }
                    }
                }
                Ok(out)
            }
        }
    })
}
