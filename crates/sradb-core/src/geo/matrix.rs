//! GEO Series Matrix: URL builder, download, gzipped TSV parser.

use std::collections::BTreeMap;
use std::io::Read;

use crate::error::{Result, SradbError};

/// Parsed `<GSE>_series_matrix.txt` content.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GeoMatrix {
    /// Lines starting with `!Series_*` or `!Sample_*`. Repeated keys are joined with `\n`.
    pub series_metadata: BTreeMap<String, String>,
    /// The TSV table between `!series_matrix_table_begin` and `!series_matrix_table_end`,
    /// including the header row.
    pub data_table: String,
}

const NCBI_GEO_FTP_HTTPS: &str = "https://ftp.ncbi.nlm.nih.gov/geo/series";

/// Compute the canonical GEO matrix URL for a `GSE<digits>` accession.
pub fn matrix_url(gse: &str) -> Result<String> {
    let acc = gse.trim();
    if !acc.starts_with("GSE") || acc.len() < 4 {
        return Err(SradbError::InvalidAccession {
            input: gse.to_owned(),
            reason: "expected GSE<digits>".into(),
        });
    }
    let digits_part = &acc[3..];
    if !digits_part.bytes().all(|b| b.is_ascii_digit()) {
        return Err(SradbError::InvalidAccession {
            input: gse.to_owned(),
            reason: "non-digit characters after GSE prefix".into(),
        });
    }
    let prefix = if digits_part.len() <= 3 {
        "GSEnnn".to_owned()
    } else {
        let head = &digits_part[..digits_part.len() - 3];
        format!("GSE{head}nnn")
    };
    Ok(format!(
        "{NCBI_GEO_FTP_HTTPS}/{prefix}/{acc}/matrix/{acc}_series_matrix.txt.gz"
    ))
}

/// Parse a (decompressed) `series_matrix.txt` body into a `GeoMatrix`.
pub fn parse_matrix(text: &str) -> Result<GeoMatrix> {
    let mut series_metadata: BTreeMap<String, String> = BTreeMap::new();
    let mut data_lines: Vec<&str> = Vec::new();
    let mut in_table = false;

    for line in text.lines() {
        if line.starts_with("!series_matrix_table_begin") {
            in_table = true;
            continue;
        }
        if line.starts_with("!series_matrix_table_end") {
            in_table = false;
            continue;
        }
        if in_table {
            data_lines.push(line);
            continue;
        }
        if let Some(rest) = line.strip_prefix('!') {
            if let Some((key, value)) = rest.split_once('\t') {
                let key = key.trim().to_owned();
                let value = value.trim_matches('"').to_owned();
                series_metadata
                    .entry(key)
                    .and_modify(|existing| {
                        existing.push('\n');
                        existing.push_str(&value);
                    })
                    .or_insert(value);
            }
        }
    }

    Ok(GeoMatrix {
        series_metadata,
        data_table: data_lines.join("\n"),
    })
}

/// Decompress gzipped bytes and parse via `parse_matrix`.
pub fn parse_matrix_gz(bytes: &[u8]) -> Result<GeoMatrix> {
    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut text = String::new();
    decoder.read_to_string(&mut text).map_err(SradbError::Io)?;
    parse_matrix(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_url_typical() {
        assert_eq!(
            matrix_url("GSE56924").unwrap(),
            "https://ftp.ncbi.nlm.nih.gov/geo/series/GSE56nnn/GSE56924/matrix/GSE56924_series_matrix.txt.gz"
        );
        assert_eq!(
            matrix_url("GSE253406").unwrap(),
            "https://ftp.ncbi.nlm.nih.gov/geo/series/GSE253nnn/GSE253406/matrix/GSE253406_series_matrix.txt.gz"
        );
    }

    #[test]
    fn matrix_url_short_accession() {
        assert_eq!(
            matrix_url("GSE1").unwrap(),
            "https://ftp.ncbi.nlm.nih.gov/geo/series/GSEnnn/GSE1/matrix/GSE1_series_matrix.txt.gz"
        );
    }

    #[test]
    fn matrix_url_invalid() {
        assert!(matrix_url("SRP174132").is_err());
        assert!(matrix_url("GSE").is_err());
        assert!(matrix_url("GSE12abc").is_err());
    }

    const SAMPLE_MATRIX: &str = "!Series_title\t\"Test study\"\n\
!Series_summary\t\"Line 1\"\n\
!Series_summary\t\"Line 2\"\n\
!Sample_title\t\"sample 1\"\t\"sample 2\"\n\
!series_matrix_table_begin\n\
\"ID_REF\"\t\"GSM1\"\t\"GSM2\"\n\
\"PROBE_A\"\t1.0\t2.0\n\
\"PROBE_B\"\t3.0\t4.0\n\
!series_matrix_table_end\n\
";

    #[test]
    fn parses_metadata_and_table() {
        let m = parse_matrix(SAMPLE_MATRIX).unwrap();
        assert_eq!(
            m.series_metadata.get("Series_title").map(String::as_str),
            Some("Test study")
        );
        let summary = m.series_metadata.get("Series_summary").unwrap();
        assert!(summary.contains("Line 1"));
        assert!(summary.contains("Line 2"));
        assert!(m.data_table.contains("ID_REF"));
        assert!(m.data_table.contains("PROBE_A"));
        assert_eq!(m.data_table.lines().count(), 3);
    }

    #[test]
    fn parses_empty_body() {
        let m = parse_matrix("").unwrap();
        assert!(m.series_metadata.is_empty());
        assert_eq!(m.data_table, "");
    }

    #[test]
    fn parses_round_trip_through_gzip() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(SAMPLE_MATRIX.as_bytes()).unwrap();
        let gz = enc.finish().unwrap();
        let m = parse_matrix_gz(&gz).unwrap();
        assert!(m.data_table.contains("PROBE_A"));
    }
}
