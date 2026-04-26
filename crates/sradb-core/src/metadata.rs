//! Metadata orchestrator: chains esearch → esummary → parse → typed `MetadataRow`.
//!
//! When `opts.detailed = true`, additional fetches augment each row with:
//! - runinfo CSV (refines `total_bases`, `total_size`, `published`)
//! - `EXPERIMENT_PACKAGE_SET` XML (sample attributes, NCBI/S3/GS download URLs)
//! - ENA filereport per run (fastq URLs, fan-out concurrency = 8)

use std::sync::Arc;

use futures::stream::{FuturesUnordered, StreamExt};
use tokio::sync::Semaphore;

use crate::error::{Result, SradbError};
use crate::http::HttpClient;
use crate::model::{MetadataOpts, MetadataRow, Run, RunUrls};
use crate::ncbi::{efetch, esearch, esummary};
use crate::{ena, parse};

const ENA_CONCURRENCY: usize = 8;

/// Drive the full metadata flow for a single accession.
pub async fn fetch_metadata(
    http: &HttpClient,
    ncbi_base_url: &str,
    ena_base_url: &str,
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

    // Default-mode rows: assemble from esummary first.
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
            rows.extend(assemble_default_rows(d)?);
        }
        retstart += page;
    }

    if !opts.detailed {
        return Ok(rows);
    }

    // Detailed-mode augmentation.
    augment_with_runinfo(
        http,
        ncbi_base_url,
        &result.webenv,
        &result.query_key,
        api_key,
        page,
        total,
        &mut rows,
    )
    .await?;
    augment_with_experiment_package(
        http,
        ncbi_base_url,
        &result.webenv,
        &result.query_key,
        api_key,
        page,
        total,
        &mut rows,
    )
    .await?;
    augment_with_ena_fastq(http, ena_base_url, &mut rows).await?;

    if opts.enrich {
        if let Some(cfg) = crate::enrich::EnrichConfig::from_env() {
            crate::enrich::enrich_rows(&cfg, &mut rows).await?;
        } else {
            return Err(crate::error::SradbError::Enrichment {
                message: "OPENAI_API_KEY not set; cannot enrich".into(),
                source: None,
            });
        }
    }

    Ok(rows)
}

pub(crate) fn assemble_default_rows(doc: parse::esummary::RawDocSum) -> Result<Vec<MetadataRow>> {
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

async fn augment_with_runinfo(
    http: &HttpClient,
    ncbi_base_url: &str,
    webenv: &str,
    query_key: &str,
    api_key: Option<&str>,
    page: u32,
    total: u32,
    rows: &mut [MetadataRow],
) -> Result<()> {
    let mut runinfo: std::collections::HashMap<String, parse::runinfo::RunInfo> =
        std::collections::HashMap::new();
    let mut retstart: u32 = 0;
    while retstart < total {
        let body = efetch::efetch_runinfo_with_history(
            http,
            ncbi_base_url,
            "sra",
            webenv,
            query_key,
            retstart,
            page,
            api_key,
        )
        .await?;
        let map = parse::runinfo::parse(&body)?;
        runinfo.extend(map);
        retstart += page;
    }
    for row in rows.iter_mut() {
        if let Some(info) = runinfo.get(&row.run.accession) {
            if let Some(b) = info.bases {
                row.run.total_bases = Some(b);
            }
            if let Some(mb) = info.size_mb {
                // size_MB is in megabytes; the public field is bytes.
                row.run.total_size = Some(mb.saturating_mul(1_000_000));
            }
            if let Some(d) = &info.release_date {
                row.run.published = Some(d.clone());
            }
        }
    }
    Ok(())
}

async fn augment_with_experiment_package(
    http: &HttpClient,
    ncbi_base_url: &str,
    webenv: &str,
    query_key: &str,
    api_key: Option<&str>,
    page: u32,
    total: u32,
    rows: &mut [MetadataRow],
) -> Result<()> {
    let mut packages: std::collections::HashMap<
        String,
        parse::experiment_package::ExperimentPackage,
    > = std::collections::HashMap::new();
    let mut retstart: u32 = 0;
    while retstart < total {
        let body = efetch::efetch_full_xml_with_history(
            http,
            ncbi_base_url,
            "sra",
            webenv,
            query_key,
            retstart,
            page,
            api_key,
        )
        .await?;
        let map = parse::experiment_package::parse(&body)?;
        packages.extend(map);
        retstart += page;
    }
    for row in rows.iter_mut() {
        if let Some(pkg) = packages.get(&row.experiment.accession) {
            // Sample attributes: convert per-experiment attrs into the row's sample.
            row.sample.attributes = pkg.sample_attributes.clone();
            // Per-run download URLs.
            if let Some(urls) = pkg.run_urls.get(&row.run.accession) {
                row.run.urls.ncbi_sra = urls.ncbi_sra.clone();
                row.run.urls.s3 = urls.s3.clone();
                row.run.urls.gs = urls.gs.clone();
            }
            // Run published (overrides default-mode fallback).
            if let Some(p) = pkg.run_published.get(&row.run.accession) {
                row.run.published = Some(p.clone());
            }
        }
    }
    Ok(())
}

async fn augment_with_ena_fastq(
    http: &HttpClient,
    ena_base_url: &str,
    rows: &mut [MetadataRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let semaphore = Arc::new(Semaphore::new(ENA_CONCURRENCY));
    let http_owned = http.clone();
    let base = ena_base_url.to_owned();
    let mut futures = FuturesUnordered::new();
    for (idx, row) in rows.iter().enumerate() {
        let semaphore = semaphore.clone();
        let http = http_owned.clone();
        let base = base.clone();
        let acc = row.run.accession.clone();
        futures.push(async move {
            let _permit = semaphore.acquire().await.expect("semaphore not closed");
            let body = ena::fetch_filereport(&http, &base, &acc).await;
            (idx, body)
        });
    }
    while let Some((idx, body)) = futures.next().await {
        let body = match body {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("ENA filereport failed for {}: {e}", rows[idx].run.accession);
                continue;
            }
        };
        let parsed = match parse::ena_filereport::parse(&body) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    "ENA filereport parse failed for {}: {e}",
                    rows[idx].run.accession
                );
                continue;
            }
        };
        if let Some(r) = parsed
            .into_iter()
            .find(|r| r.run_accession == rows[idx].run.accession)
        {
            rows[idx].run.urls.ena_fastq_ftp = r
                .fastq_ftp
                .iter()
                .map(|p| {
                    if p.starts_with("ftp://") || p.starts_with("http") {
                        p.clone()
                    } else {
                        format!("ftp://{p}")
                    }
                })
                .collect();
            rows[idx].run.urls.ena_fastq_http = r
                .fastq_ftp
                .iter()
                .map(|p| {
                    if p.starts_with("http") {
                        p.clone()
                    } else {
                        let trimmed = p.strip_prefix("ftp://").unwrap_or(p);
                        format!("https://{trimmed}")
                    }
                })
                .collect();
        }
    }
    Ok(())
}
