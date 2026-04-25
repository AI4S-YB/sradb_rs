//! Metadata orchestrator: chains esearch → esummary → parse → typed `MetadataRow`.
//!
//! Slice 2 implements the default (non-detailed) path. `--detailed` and `--enrich`
//! land in slices 3 and 7 respectively.

use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::model::{MetadataOpts, MetadataRow, Run, RunUrls};
use crate::ncbi::{esearch, esummary};
use crate::parse;

/// Drive the full default-metadata flow for a single accession.
///
/// Pagination: if the esearch count exceeds `opts.page_size`, esummary is called
/// repeatedly with increasing `retstart` until all rows are collected.
pub async fn fetch_metadata(
    http: &HttpClient,
    ncbi_base_url: &str,
    api_key: Option<&str>,
    term: &str,
    opts: &MetadataOpts,
) -> Result<Vec<MetadataRow>> {
    let page = opts.page_size.max(1);
    let result = esearch::esearch(http, ncbi_base_url, "sra", term, api_key, page).await?;
    if result.count == 0 {
        return Ok(Vec::new());
    }
    if result.webenv.is_empty() || result.query_key.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "esearch",
            message: format!("count={} but missing webenv/query_key", result.count),
        });
    }

    let mut rows: Vec<MetadataRow> = Vec::with_capacity(result.count as usize);
    let mut retstart: u32 = 0;
    let total = u32::try_from(result.count).unwrap_or(u32::MAX);
    while retstart < total {
        let body = esummary::esummary_with_history(
            http,
            ncbi_base_url,
            "sra",
            &result.webenv,
            &result.query_key,
            retstart,
            page,
            api_key,
        )
        .await?;
        let docs = parse::esummary::parse(&body)?;
        if docs.is_empty() {
            break;
        }
        for d in docs {
            rows.extend(assemble_rows(d)?);
        }
        retstart += page;
    }
    Ok(rows)
}

/// One `DocSum` can carry multiple `<Run>` entries (paired-end studies, etc.).
/// Emit one `MetadataRow` per run, sharing the experiment/study/sample.
fn assemble_rows(doc: parse::esummary::RawDocSum) -> Result<Vec<MetadataRow>> {
    let exp = parse::exp_xml::parse(&doc.exp_xml)?;
    let runs = parse::exp_xml::parse_runs(&doc.runs)?;
    if runs.is_empty() {
        return Err(SradbError::Parse {
            endpoint: "esummary",
            message: format!("no <Run> in DocSum id={}", doc.id),
        });
    }
    let (experiment, study, sample) = parse::exp_xml::project(exp.clone());
    let published = doc.update_date.or(doc.create_date);
    let rows = runs
        .into_iter()
        .map(|raw_run| MetadataRow {
            run: Run {
                accession: raw_run.accession,
                experiment_accession: experiment.accession.clone(),
                sample_accession: experiment.sample_accession.clone(),
                study_accession: experiment.study_accession.clone(),
                total_spots: raw_run.total_spots.or(exp.total_spots),
                total_bases: raw_run.total_bases.or(exp.total_bases),
                total_size: exp.total_size,
                published: published.clone(),
                urls: RunUrls::default(),
            },
            experiment: experiment.clone(),
            sample: sample.clone(),
            study: study.clone(),
            enrichment: None,
        })
        .collect();
    Ok(rows)
}
