//! Parser for the efetch retmode=runinfo CSV output.
//!
//! NCBI's eUtils returns runinfo in two flavors:
//!  - header-bearing CSV (when fetched directly with `?id=...`)
//!  - headerless CSV (when fetched with `usehistory=y` + `WebEnv`, as our orchestrator does)
//!
//! We auto-detect by looking at the first row: if its first cell starts with
//! `Run` (case-insensitive), it's a header. Otherwise we treat all rows as data
//! and use NCBI's canonical positional column layout (46 columns, stable for
//! years).
//!
//! We only consume the four columns needed to refine `Run` fields beyond what
//! `ExpXml` provided.

use std::collections::HashMap;

use crate::error::{Result, SradbError};

const CONTEXT: &str = "efetch_runinfo";

// NCBI canonical runinfo column positions (0-indexed).
const POS_RUN: usize = 0;
const POS_RELEASE_DATE: usize = 1;
const POS_BASES: usize = 4;
const POS_SIZE_MB: usize = 7;

/// Per-run augmentation extracted from runinfo CSV.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RunInfo {
    pub run_accession: String,
    pub bases: Option<u64>,
    pub size_mb: Option<u64>,
    pub release_date: Option<String>,
}

/// Parse a runinfo CSV body into a map keyed by run accession.
pub fn parse(body: &str) -> Result<HashMap<String, RunInfo>> {
    let trimmed = body.trim_start();
    if trimmed.is_empty() {
        return Ok(HashMap::new());
    }

    // Peek the first non-empty line to determine whether a header row is present.
    let first_line = trimmed.lines().next().unwrap_or("");
    let first_cell = first_line.split(',').next().unwrap_or("").trim();
    let has_header = first_cell.eq_ignore_ascii_case("Run");

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(has_header)
        .flexible(true)
        .from_reader(body.as_bytes());

    let (i_run, i_release, i_bases, i_size_mb) = if has_header {
        let headers = reader
            .headers()
            .map_err(|e| SradbError::Csv {
                context: CONTEXT,
                source: e,
            })?
            .clone();
        let col = |name: &str| headers.iter().position(|h| h.eq_ignore_ascii_case(name));
        (
            col("Run").unwrap_or(POS_RUN),
            col("ReleaseDate").unwrap_or(POS_RELEASE_DATE),
            col("bases").unwrap_or(POS_BASES),
            col("size_MB")
                .or_else(|| col("size_mb"))
                .unwrap_or(POS_SIZE_MB),
        )
    } else {
        (POS_RUN, POS_RELEASE_DATE, POS_BASES, POS_SIZE_MB)
    };

    let mut out: HashMap<String, RunInfo> = HashMap::new();
    for record in reader.records() {
        let record = record.map_err(|e| SradbError::Csv {
            context: CONTEXT,
            source: e,
        })?;
        let mut info = RunInfo::default();
        if let Some(v) = record.get(i_run) {
            info.run_accession.clear();
            info.run_accession.push_str(v);
        }
        if info.run_accession.is_empty() {
            continue;
        }
        info.bases = record.get(i_bases).and_then(|s| s.parse().ok());
        info.size_mb = record.get(i_size_mb).and_then(|s| s.parse().ok());
        info.release_date = record
            .get(i_release)
            .map(str::to_owned)
            .filter(|s| !s.is_empty());
        out.insert(info.run_accession.clone(), info);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_WITH_HEADER: &str = "Run,ReleaseDate,LoadDate,spots,bases,spots_with_mates,avgLength,size_MB,Experiment\nSRR8361601,2018-12-20 10:00:00,2018-12-21,38671668,11678843736,0,302,4894,SRX5172107\n";

    const SAMPLE_HEADERLESS: &str = "SRR8361592,2019-11-21 00:48:25,2018-12-20 17:28:37,35353289,10676693278,35353289,302,4494,,https://example/SRR8361592.sra,SRX5172098\n";

    #[test]
    fn parses_with_header() {
        let map = parse(SAMPLE_WITH_HEADER).unwrap();
        assert_eq!(map.len(), 1);
        let info = map.get("SRR8361601").unwrap();
        assert_eq!(info.bases, Some(11_678_843_736));
        assert_eq!(info.size_mb, Some(4894));
        assert_eq!(info.release_date.as_deref(), Some("2018-12-20 10:00:00"));
    }

    #[test]
    fn parses_headerless() {
        let map = parse(SAMPLE_HEADERLESS).unwrap();
        assert_eq!(map.len(), 1);
        let info = map.get("SRR8361592").unwrap();
        assert_eq!(info.bases, Some(10_676_693_278));
        assert_eq!(info.size_mb, Some(4494));
        assert_eq!(info.release_date.as_deref(), Some("2019-11-21 00:48:25"));
    }

    #[test]
    fn parses_real_srp174132_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_runinfo_SRP174132.csv"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-efetch-runinfo SRP174132` first");
        let map = parse(&body).unwrap();
        assert_eq!(map.len(), 10, "SRP174132 has 10 runs");
        for (acc, info) in &map {
            assert!(acc.starts_with("SRR"));
            assert_eq!(&info.run_accession, acc);
            assert!(info.bases.is_some(), "bases should parse for {acc}");
            assert!(info.size_mb.is_some(), "size_MB should parse for {acc}");
            assert!(
                info.release_date.is_some(),
                "release_date should parse for {acc}"
            );
        }
    }
}
