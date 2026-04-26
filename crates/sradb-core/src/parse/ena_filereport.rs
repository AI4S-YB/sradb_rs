//! Parser for ENA filereport TSV (`/portal/api/filereport`).
//!
//! Each row maps a run accession to its fastq URLs (FTP and aspera) plus
//! md5/byte sizes. For paired-end runs, the `fastq_ftp` field holds two
//! `;`-separated paths; we split into per-mate vectors.

use crate::error::{Result, SradbError};

const CONTEXT: &str = "ena_filereport";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnaFilereportRow {
    pub run_accession: String,
    pub fastq_ftp: Vec<String>, // 0..2 entries
    pub fastq_md5: Vec<String>,
    pub fastq_bytes: Vec<u64>,
    pub fastq_aspera: Vec<String>,
}

/// Parse an ENA filereport TSV body. Empty body → empty vec.
pub fn parse(body: &str) -> Result<Vec<EnaFilereportRow>> {
    if body.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .delimiter(b'\t')
        .flexible(true)
        .from_reader(body.as_bytes());

    let headers = reader
        .headers()
        .map_err(|e| SradbError::Csv {
            context: CONTEXT,
            source: e,
        })?
        .clone();
    let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
    let i_run = col("run_accession");
    let i_ftp = col("fastq_ftp");
    let i_md5 = col("fastq_md5");
    let i_bytes = col("fastq_bytes");
    let i_aspera = col("fastq_aspera");

    let mut out = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| SradbError::Csv {
            context: CONTEXT,
            source: e,
        })?;
        let mut row = EnaFilereportRow::default();
        if let Some(i) = i_run {
            row.run_accession.clear();
            row.run_accession
                .push_str(record.get(i).unwrap_or_default());
        }
        if row.run_accession.is_empty() {
            continue;
        }
        row.fastq_ftp = split_semi(record.get(i_ftp.unwrap_or(usize::MAX)));
        row.fastq_md5 = split_semi(record.get(i_md5.unwrap_or(usize::MAX)));
        row.fastq_bytes = split_semi(record.get(i_bytes.unwrap_or(usize::MAX)))
            .into_iter()
            .filter_map(|s| s.parse().ok())
            .collect();
        row.fastq_aspera = split_semi(record.get(i_aspera.unwrap_or(usize::MAX)));
        out.push(row);
    }
    Ok(out)
}

fn split_semi(s: Option<&str>) -> Vec<String> {
    s.unwrap_or("")
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "run_accession\tfastq_ftp\tfastq_md5\tfastq_bytes\tfastq_aspera\n\
SRR8361601\tftp.sra.ebi.ac.uk/vol1/fastq/SRR836/001/SRR8361601/SRR8361601_1.fastq.gz;ftp.sra.ebi.ac.uk/vol1/fastq/SRR836/001/SRR8361601/SRR8361601_2.fastq.gz\tabc;def\t1234567;7654321\tera-fasp@x;era-fasp@y\n";

    #[test]
    fn parses_paired_end() {
        let rows = parse(SAMPLE).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.run_accession, "SRR8361601");
        assert_eq!(r.fastq_ftp.len(), 2);
        assert!(r.fastq_ftp[0].ends_with("_1.fastq.gz"));
        assert!(r.fastq_ftp[1].ends_with("_2.fastq.gz"));
        assert_eq!(r.fastq_md5, vec!["abc".to_string(), "def".into()]);
        assert_eq!(r.fastq_bytes, vec![1_234_567, 7_654_321]);
    }

    #[test]
    fn parses_real_srr8361601_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ena/filereport_SRR8361601.tsv"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-ena-filereport SRR8361601` first");
        let rows = parse(&body).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.run_accession, "SRR8361601");
        assert!(
            !r.fastq_ftp.is_empty(),
            "should have at least one fastq URL"
        );
    }

    #[test]
    fn empty_body_yields_empty_vec() {
        assert!(parse("").unwrap().is_empty());
    }
}
