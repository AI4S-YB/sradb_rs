//! Output writers for `Vec<MetadataRow>`.

use std::io::{self, Write};

use sradb_core::MetadataRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Tsv,
    Json,
    Ndjson,
}

const TSV_COLUMNS: &[&str] = &[
    "study_accession",
    "study_title",
    "experiment_accession",
    "experiment_title",
    "organism_taxid",
    "organism_name",
    "library_strategy",
    "library_source",
    "library_selection",
    "library_layout",
    "sample_accession",
    "sample_title",
    "biosample",
    "bioproject",
    "instrument",
    "instrument_model",
    "total_spots",
    "total_bases",
    "total_size",
    "run_accession",
    "run_total_spots",
    "run_total_bases",
];

fn cell(row: &MetadataRow, col: &str) -> String {
    use sradb_core::LibraryLayout;
    let opt_string = |s: &Option<String>| s.clone().unwrap_or_default();
    let opt_num = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_default();
    match col {
        "study_accession" => row.study.accession.clone(),
        "study_title" => opt_string(&row.study.title),
        "experiment_accession" => row.experiment.accession.clone(),
        "experiment_title" => opt_string(&row.experiment.title),
        "organism_taxid" => row
            .sample
            .organism_taxid
            .map(|n| n.to_string())
            .unwrap_or_default(),
        "organism_name" => opt_string(&row.sample.organism_name),
        "library_strategy" => opt_string(&row.experiment.library.strategy),
        "library_source" => opt_string(&row.experiment.library.source),
        "library_selection" => opt_string(&row.experiment.library.selection),
        "library_layout" => match &row.experiment.library.layout {
            Some(LibraryLayout::Single { .. }) => "SINGLE".into(),
            Some(LibraryLayout::Paired { .. }) => "PAIRED".into(),
            Some(LibraryLayout::Unknown) | None => String::new(),
        },
        "sample_accession" => row.sample.accession.clone(),
        "sample_title" => opt_string(&row.sample.title),
        "biosample" => opt_string(&row.sample.biosample),
        "bioproject" => opt_string(&row.study.bioproject),
        "instrument" => opt_string(&row.experiment.platform.name),
        "instrument_model" => opt_string(&row.experiment.platform.instrument_model),
        "total_spots" | "run_total_spots" => opt_num(row.run.total_spots),
        "total_bases" | "run_total_bases" => opt_num(row.run.total_bases),
        "total_size" => opt_num(row.run.total_size),
        "run_accession" => row.run.accession.clone(),
        _ => String::new(),
    }
}

pub fn write(rows: &[MetadataRow], format: Format, mut out: impl Write) -> io::Result<()> {
    match format {
        Format::Tsv => write_tsv(rows, &mut out),
        Format::Json => write_json(rows, &mut out),
        Format::Ndjson => write_ndjson(rows, &mut out),
    }
}

fn write_tsv<W: Write>(rows: &[MetadataRow], out: &mut W) -> io::Result<()> {
    writeln!(out, "{}", TSV_COLUMNS.join("\t"))?;
    for row in rows {
        let cells: Vec<String> = TSV_COLUMNS
            .iter()
            .map(|c| sanitize_tsv(&cell(row, c)))
            .collect();
        writeln!(out, "{}", cells.join("\t"))?;
    }
    Ok(())
}

fn sanitize_tsv(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

fn write_json<W: Write>(rows: &[MetadataRow], out: &mut W) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *out, rows).map_err(io::Error::other)?;
    writeln!(out)?;
    Ok(())
}

fn write_ndjson<W: Write>(rows: &[MetadataRow], out: &mut W) -> io::Result<()> {
    for row in rows {
        serde_json::to_writer(&mut *out, row).map_err(io::Error::other)?;
        writeln!(out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sradb_core::{Experiment, Library, MetadataRow, Platform, Run, RunUrls, Sample, Study};

    fn fixture_row() -> MetadataRow {
        MetadataRow {
            run: Run {
                accession: "SRR8361601".into(),
                experiment_accession: "SRX5172107".into(),
                sample_accession: "SRS4179725".into(),
                study_accession: "SRP174132".into(),
                total_spots: Some(38_671_668),
                total_bases: Some(11_678_843_736),
                total_size: Some(5_132_266_976),
                published: None,
                urls: RunUrls::default(),
            },
            experiment: Experiment {
                accession: "SRX5172107".into(),
                title: Some("RNA-Seq: H1".into()),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                design_description: None,
                library: Library {
                    strategy: Some("RNA-Seq".into()),
                    source: Some("TRANSCRIPTOMIC".into()),
                    selection: Some("cDNA".into()),
                    layout: Some(sradb_core::LibraryLayout::Paired {
                        nominal_length: None,
                        nominal_sdev: None,
                    }),
                    construction_protocol: None,
                },
                platform: Platform {
                    name: Some("ILLUMINA".into()),
                    instrument_model: Some("Illumina HiSeq 2000".into()),
                },
                geo_accession: None,
            },
            sample: Sample {
                accession: "SRS4179725".into(),
                title: None,
                biosample: Some("SAMN10621858".into()),
                organism_taxid: Some(9606),
                organism_name: Some("Homo sapiens".into()),
                attributes: Default::default(),
            },
            study: Study {
                accession: "SRP174132".into(),
                title: Some("ARID1A study".into()),
                abstract_: None,
                bioproject: Some("PRJNA511021".into()),
                geo_accession: None,
                pmids: vec![],
            },
            enrichment: None,
        }
    }

    #[test]
    fn tsv_has_header_and_one_row() {
        let mut out = Vec::new();
        write(std::slice::from_ref(&fixture_row()), Format::Tsv, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], TSV_COLUMNS.join("\t"));
        assert!(lines[1].contains("SRP174132"));
        assert!(lines[1].contains("SRR8361601"));
        assert!(lines[1].contains("RNA-Seq"));
        assert!(lines[1].contains("PAIRED"));
    }

    #[test]
    fn json_round_trips() {
        let mut out = Vec::new();
        write(std::slice::from_ref(&fixture_row()), Format::Json, &mut out).unwrap();
        let s = String::from_utf8(out).unwrap();
        let back: Vec<MetadataRow> = serde_json::from_str(&s).unwrap();
        assert_eq!(back, vec![fixture_row()]);
    }

    #[test]
    fn ndjson_has_one_line_per_row() {
        let row = fixture_row();
        let rows = vec![row.clone(), row];
        let mut out = Vec::new();
        write(&rows, Format::Ndjson, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(text.lines().count(), 2);
        for line in text.lines() {
            let _: MetadataRow = serde_json::from_str(line).unwrap();
        }
    }
}
