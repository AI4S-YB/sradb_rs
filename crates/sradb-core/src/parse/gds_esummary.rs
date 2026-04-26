//! Parser for NCBI db=gds esummary JSON responses.
//!
//! The shape is `{"result": {"uids": [...], "<uid>": { ... }, ...}}`. We project
//! the fields needed for accession conversion: `accession`, `entrytype`,
//! `samples` (children for GSEs), and `extrelations` (cross-DB links).

use serde::Deserialize;

use crate::error::{Result, SradbError};

const CONTEXT: &str = "gds_esummary";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdsRecord {
    pub uid: String,
    pub accession: String,
    pub entry_type: String, // "GSE", "GSM", "GPL"
    pub n_samples: Option<u32>,
    pub samples: Vec<GdsSample>,
    pub extrelations: Vec<GdsExtRelation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GdsSample {
    #[serde(default)]
    pub accession: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GdsExtRelation {
    #[serde(default, rename = "relationtype")]
    pub relation_type: String, // typically "SRA"
    #[serde(default, rename = "targetobject")]
    pub target_object: String, // typically an SRP accession (for GSE) or SRX (for GSM)
}

/// Parse a db=gds esummary JSON body into one record per UID.
pub fn parse(body: &str) -> Result<Vec<GdsRecord>> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|source| SradbError::Json {
        context: CONTEXT,
        source,
    })?;
    let result = v.get("result").ok_or_else(|| SradbError::Parse {
        endpoint: CONTEXT,
        message: "missing `result` field".into(),
    })?;
    let uids = result
        .get("uids")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| SradbError::Parse {
            endpoint: CONTEXT,
            message: "missing `result.uids` array".into(),
        })?;

    let mut out = Vec::with_capacity(uids.len());
    for uid_v in uids {
        let uid = match uid_v.as_str() {
            Some(s) => s.to_owned(),
            None => continue,
        };
        let record = match result.get(&uid) {
            Some(r) => r,
            None => continue,
        };

        let accession = record.get("accession").and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let entry_type = record.get("entrytype").and_then(|x| x.as_str()).unwrap_or("").to_owned();
        let n_samples = record.get("n_samples").and_then(serde_json::Value::as_u64).map(|n| n as u32);

        let samples: Vec<GdsSample> = record
            .get("samples")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| serde_json::from_value::<GdsSample>(s.clone()).ok())
                    .filter(|s| !s.accession.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let extrelations: Vec<GdsExtRelation> = record
            .get("extrelations")
            .and_then(serde_json::Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| serde_json::from_value::<GdsExtRelation>(r.clone()).ok())
                    .filter(|r| !r.target_object.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        out.push(GdsRecord {
            uid,
            accession,
            entry_type,
            n_samples,
            samples,
            extrelations,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_GSE: &str = r#"{
"header":{"type":"esummary","version":"0.3"},
"result":{"uids":["200056924"],
"200056924":{"uid":"200056924","accession":"GSE56924","entrytype":"GSE","n_samples":96,
"samples":[{"accession":"GSM1371490","title":"sample 1"},{"accession":"GSM1371491","title":"sample 2"}],
"extrelations":[{"relationtype":"SRA","targetobject":"SRP041298","targetftplink":"ftp://..."}]}
}}"#;

    const SAMPLE_GSM: &str = r#"{
"header":{"type":"esummary"},
"result":{"uids":["301371490"],
"301371490":{"uid":"301371490","accession":"GSM1371490","entrytype":"GSM","n_samples":0,
"extrelations":[{"relationtype":"SRA","targetobject":"SRX522504"}]}
}}"#;

    #[test]
    fn parses_gse_record() {
        let recs = parse(SAMPLE_GSE).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.accession, "GSE56924");
        assert_eq!(r.entry_type, "GSE");
        assert_eq!(r.n_samples, Some(96));
        assert_eq!(r.samples.len(), 2);
        assert_eq!(r.samples[0].accession, "GSM1371490");
        assert_eq!(r.extrelations.len(), 1);
        assert_eq!(r.extrelations[0].relation_type, "SRA");
        assert_eq!(r.extrelations[0].target_object, "SRP041298");
    }

    #[test]
    fn parses_gsm_record() {
        let recs = parse(SAMPLE_GSM).unwrap();
        assert_eq!(recs.len(), 1);
        let r = &recs[0];
        assert_eq!(r.accession, "GSM1371490");
        assert_eq!(r.entry_type, "GSM");
        assert_eq!(r.extrelations[0].target_object, "SRX522504");
    }

    #[test]
    fn parses_real_gse56924_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/gds_esummary_GSE56924.json"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-gds-esummary GSE56924` first");
        let recs = parse(&body).unwrap();
        assert!(!recs.is_empty());
        let r = recs.iter().find(|r| r.accession == "GSE56924").expect("GSE56924 record");
        assert_eq!(r.entry_type, "GSE");
        assert!(r.n_samples.unwrap_or(0) > 0);
        assert!(!r.samples.is_empty());
        assert!(r.extrelations.iter().any(|e| e.target_object.starts_with("SRP")));
    }

    #[test]
    fn parses_real_gsm1371490_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/gds_esummary_GSM1371490.json"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-gds-esummary GSM1371490` first");
        let recs = parse(&body).unwrap();
        let r = recs.iter().find(|r| r.accession == "GSM1371490").expect("GSM1371490 record");
        assert_eq!(r.entry_type, "GSM");
    }
}
