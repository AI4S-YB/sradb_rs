//! Parser for the `EXPERIMENT_PACKAGE_SET` XML returned by `efetch retmode=xml`.
//!
//! Slice 3 extracts: per-experiment `SAMPLE_ATTRIBUTES` (key/value bag) and
//! per-run `SRAFile` alternatives (NCBI / S3 / GS download URLs). Task 5
//! extends this parser with the `SRAFile` path; Task 4 lands sample attributes.

use std::collections::BTreeMap;
use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Result, SradbError};

const CONTEXT: &str = "efetch_xml";

/// Per-experiment data extracted from one `<EXPERIMENT_PACKAGE>`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExperimentPackage {
    pub experiment_accession: String,
    pub sample_accession: String,
    pub sample_attributes: BTreeMap<String, String>,
    /// Download URLs by run accession.
    pub run_urls: HashMap<String, SraFileUrls>,
    /// Run published timestamp (overrides default-mode fallback).
    pub run_published: HashMap<String, String>,
}

/// Per-run download URLs extracted from `<SRAFiles>/<SRAFile>/<Alternatives>`.
/// Distinct from `model::RunUrls` (which also carries ENA fastq lists).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SraFileUrls {
    pub ncbi_sra: Option<String>,
    pub s3: Option<String>,
    pub gs: Option<String>,
}

/// Parse an entire `EXPERIMENT_PACKAGE_SET` body into one `ExperimentPackage` per
/// experiment, keyed by experiment accession.
pub fn parse(body: &str) -> Result<HashMap<String, ExperimentPackage>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    let mut packages: HashMap<String, ExperimentPackage> = HashMap::new();
    let mut current: Option<ExperimentPackage> = None;

    // SAMPLE_ATTRIBUTE tracking
    let mut in_sample = false;
    let mut in_sample_attributes = false;
    let mut in_sample_attribute = false;
    let mut tag_text: Option<String> = None;
    let mut value_text: Option<String> = None;
    let mut text_target: Option<TextTarget> = None;
    let mut text_buf = String::new();

    // RUN / SRAFiles tracking
    let mut current_run_acc: Option<String> = None;
    let mut current_run_published: Option<String> = None;
    let mut current_sra_file_alternatives: Vec<(String, String)> = Vec::new(); // (org, url) per run
    let mut in_run = false;
    let mut in_sra_files = false;
    let mut in_sra_file = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return Err(SradbError::Xml {
                    context: CONTEXT,
                    source: e,
                })
            }
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e) | Event::Start(e)) => match e.name().as_ref() {
                b"EXPERIMENT_PACKAGE" => current = Some(ExperimentPackage::default()),
                b"EXPERIMENT" => {
                    if let Some(p) = current.as_mut() {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"accession" {
                                let v = attr
                                    .unescape_value()
                                    .map_err(|e| SradbError::Xml {
                                        context: CONTEXT,
                                        source: e,
                                    })?
                                    .into_owned();
                                p.experiment_accession = v;
                            }
                        }
                    }
                }
                b"SAMPLE" => {
                    in_sample = true;
                    if let Some(p) = current.as_mut() {
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"accession" {
                                let v = attr
                                    .unescape_value()
                                    .map_err(|e| SradbError::Xml {
                                        context: CONTEXT,
                                        source: e,
                                    })?
                                    .into_owned();
                                p.sample_accession = v;
                            }
                        }
                    }
                }
                b"SAMPLE_ATTRIBUTES" if in_sample => in_sample_attributes = true,
                b"SAMPLE_ATTRIBUTE" if in_sample_attributes => {
                    in_sample_attribute = true;
                    tag_text = None;
                    value_text = None;
                }
                b"TAG" if in_sample_attribute => {
                    text_buf.clear();
                    text_target = Some(TextTarget::Tag);
                }
                b"VALUE" if in_sample_attribute => {
                    text_buf.clear();
                    text_target = Some(TextTarget::Value);
                }
                b"RUN" => {
                    in_run = true;
                    current_run_acc = None;
                    current_run_published = None;
                    current_sra_file_alternatives.clear();
                    for attr in e.attributes().flatten() {
                        let val = attr.unescape_value().map_err(|e| SradbError::Xml {
                            context: CONTEXT,
                            source: e,
                        })?;
                        match attr.key.as_ref() {
                            b"accession" => current_run_acc = Some(val.into_owned()),
                            b"published" => current_run_published = Some(val.into_owned()),
                            _ => {}
                        }
                    }
                }
                b"SRAFiles" if in_run => in_sra_files = true,
                b"SRAFile" if in_sra_files => in_sra_file = true,
                b"Alternatives" if in_sra_file => {
                    let mut org: Option<String> = None;
                    let mut url: Option<String> = None;
                    for attr in e.attributes().flatten() {
                        let v = attr.unescape_value().map_err(|e| SradbError::Xml {
                            context: CONTEXT,
                            source: e,
                        })?;
                        match attr.key.as_ref() {
                            b"org" => org = Some(v.into_owned()),
                            b"url" => url = Some(v.into_owned()),
                            _ => {}
                        }
                    }
                    if let (Some(org), Some(url)) = (org, url) {
                        current_sra_file_alternatives.push((org, url));
                    }
                }
                _ => {}
            },
            Ok(Event::Text(e)) if text_target.is_some() => {
                let s = e.unescape().map_err(|e| SradbError::Xml {
                    context: CONTEXT,
                    source: e,
                })?;
                text_buf.push_str(&s);
            }
            Ok(Event::End(e)) => {
                match e.name().as_ref() {
                    b"EXPERIMENT_PACKAGE" => {
                        if let Some(pkg) = current.take() {
                            if !pkg.experiment_accession.is_empty() {
                                packages.insert(pkg.experiment_accession.clone(), pkg);
                            }
                        }
                    }
                    b"SAMPLE" => in_sample = false,
                    b"SAMPLE_ATTRIBUTES" => in_sample_attributes = false,
                    b"SAMPLE_ATTRIBUTE" => {
                        if let (Some(t), Some(v), Some(p)) =
                            (tag_text.take(), value_text.take(), current.as_mut())
                        {
                            let t = t.trim().to_owned();
                            let v = v.trim().to_owned();
                            if !t.is_empty() {
                                p.sample_attributes.insert(t, v);
                            }
                        }
                        in_sample_attribute = false;
                    }
                    b"RUN" => {
                        if let (Some(acc), Some(p)) = (current_run_acc.take(), current.as_mut()) {
                            let mut urls = SraFileUrls::default();
                            for (org, url) in current_sra_file_alternatives.drain(..) {
                                match org.as_str() {
                                    "NCBI" => urls.ncbi_sra = Some(url),
                                    "AWS" => urls.s3 = Some(url),
                                    "GCP" => urls.gs = Some(url),
                                    _ => {}
                                }
                            }
                            p.run_urls.insert(acc.clone(), urls);
                            if let Some(pub_) = current_run_published.take() {
                                p.run_published.insert(acc, pub_);
                            }
                        }
                        in_run = false;
                    }
                    b"SRAFiles" => in_sra_files = false,
                    b"SRAFile" => in_sra_file = false,
                    _ => {}
                }
                if let Some(target) = text_target.take() {
                    let value = std::mem::take(&mut text_buf);
                    match target {
                        TextTarget::Tag => tag_text = Some(value),
                        TextTarget::Value => value_text = Some(value),
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(packages)
}

#[derive(Debug, Clone, Copy)]
enum TextTarget {
    Tag,
    Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<EXPERIMENT_PACKAGE_SET>
<EXPERIMENT_PACKAGE>
<EXPERIMENT accession="SRX5172107"/>
<SAMPLE accession="SRS4179725">
<SAMPLE_ATTRIBUTES>
<SAMPLE_ATTRIBUTE><TAG>source_name</TAG><VALUE>liver</VALUE></SAMPLE_ATTRIBUTE>
<SAMPLE_ATTRIBUTE><TAG>cell type</TAG><VALUE>hepatocyte</VALUE></SAMPLE_ATTRIBUTE>
</SAMPLE_ATTRIBUTES>
</SAMPLE>
</EXPERIMENT_PACKAGE>
</EXPERIMENT_PACKAGE_SET>"#;

    #[test]
    fn parses_sample_attributes() {
        let pkgs = parse(SAMPLE).unwrap();
        assert_eq!(pkgs.len(), 1);
        let p = &pkgs["SRX5172107"];
        assert_eq!(p.sample_accession, "SRS4179725");
        assert_eq!(
            p.sample_attributes.get("source_name").map(String::as_str),
            Some("liver")
        );
        assert_eq!(
            p.sample_attributes.get("cell type").map(String::as_str),
            Some("hepatocyte")
        );
    }

    #[test]
    fn parses_real_srp174132_fixture_sample_attrs() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_xml_SRP174132.xml"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-efetch-xml SRP174132` first");
        let pkgs = parse(&body).unwrap();
        assert!(!pkgs.is_empty(), "should have ≥ 1 package");
        for (exp, pkg) in &pkgs {
            assert!(exp.starts_with("SRX"), "experiment accession: {exp}");
            assert!(
                !pkg.sample_accession.is_empty(),
                "{exp} should have sample acc"
            );
            assert!(
                !pkg.sample_attributes.is_empty(),
                "{exp} should have sample attrs"
            );
        }
    }

    #[test]
    fn parses_sra_file_alternatives() {
        const XML: &str = r#"<?xml version="1.0"?>
<EXPERIMENT_PACKAGE_SET>
<EXPERIMENT_PACKAGE>
<EXPERIMENT accession="SRX1"/>
<SAMPLE accession="SRS1"><SAMPLE_ATTRIBUTES><SAMPLE_ATTRIBUTE><TAG>k</TAG><VALUE>v</VALUE></SAMPLE_ATTRIBUTE></SAMPLE_ATTRIBUTES></SAMPLE>
<RUN_SET>
<RUN accession="SRR1" published="2024-01-02 03:04:05">
<SRAFiles>
<SRAFile cluster="public" filename="SRR1" url="https://sra-pub.s3.amazonaws.com/SRR1" semantic_name="run">
<Alternatives url="https://sra-download.ncbi.nlm.nih.gov/traces/sra/SRR1" org="NCBI"/>
<Alternatives url="s3://sra-pub-run-odp/sra/SRR1" org="AWS"/>
<Alternatives url="gs://sra-pub-run-1/SRR1" org="GCP"/>
</SRAFile>
</SRAFiles>
</RUN>
</RUN_SET>
</EXPERIMENT_PACKAGE>
</EXPERIMENT_PACKAGE_SET>"#;
        let pkgs = parse(XML).unwrap();
        let p = &pkgs["SRX1"];
        let urls = &p.run_urls["SRR1"];
        assert_eq!(
            urls.ncbi_sra.as_deref(),
            Some("https://sra-download.ncbi.nlm.nih.gov/traces/sra/SRR1")
        );
        assert_eq!(urls.s3.as_deref(), Some("s3://sra-pub-run-odp/sra/SRR1"));
        assert_eq!(urls.gs.as_deref(), Some("gs://sra-pub-run-1/SRR1"));
        assert_eq!(
            p.run_published.get("SRR1").map(String::as_str),
            Some("2024-01-02 03:04:05")
        );
    }

    #[test]
    fn real_srp174132_fixture_has_urls() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/efetch_xml_SRP174132.xml"),
        )
        .expect("fixture missing");
        let pkgs = parse(&body).unwrap();
        let mut runs_with_any_url = 0;
        for pkg in pkgs.values() {
            for urls in pkg.run_urls.values() {
                if urls.ncbi_sra.is_some() || urls.s3.is_some() || urls.gs.is_some() {
                    runs_with_any_url += 1;
                }
            }
        }
        assert!(
            runs_with_any_url > 0,
            "at least one run should have a download URL"
        );
    }
}
