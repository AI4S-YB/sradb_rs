//! Parser for the decoded `ExpXml` fragment (and Runs fragment) returned by NCBI esummary.
//!
//! These fragments are not single-rooted XML — they are a sequence of sibling
//! elements. We wrap them with a synthetic `<Root>` before feeding to quick-xml.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Result, SradbError};
use crate::model::{Experiment, Library, LibraryLayout, Platform, Sample, Study};

const CONTEXT: &str = "esummary_exp_xml";

/// Combined payload extracted from one `ExpXml` fragment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExpXmlData {
    pub experiment_title: Option<String>,
    pub experiment_accession: String,
    pub experiment_status: Option<String>,
    pub study_accession: String,
    pub study_title: Option<String>,
    pub sample_accession: String,
    pub sample_name: Option<String>,
    pub bioproject: Option<String>,
    pub biosample: Option<String>,
    pub organism_taxid: Option<u32>,
    pub organism_name: Option<String>,
    pub platform: Platform,
    pub library: Library,
    pub total_runs: Option<u32>,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub total_size: Option<u64>,
}

/// Parse one ExpXml fragment.
pub fn parse(fragment: &str) -> Result<ExpXmlData> {
    let wrapped = format!("<Root>{fragment}</Root>");
    let mut reader = Reader::from_str(&wrapped);
    reader.config_mut().trim_text(true);

    let mut data = ExpXmlData::default();
    let mut buf = Vec::new();
    let mut text_target: Option<TextTarget> = None;
    let mut text_buf = String::new();
    let mut in_library_descriptor = false;
    let mut in_library_layout = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SradbError::Xml { context: CONTEXT, source: e }),
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                // Note: in the real eSummary payloads, the elements that we set
                // `text_target` on (Title, Platform, Bioproject, Biosample,
                // LIBRARY_*) ALWAYS arrive as Event::Start (they have content).
                // Attribute-only elements like <Statistics ... />, <Experiment ... />,
                // <Sample ... />, <Organism ... />, <PAIRED/> arrive as Event::Empty.
                // Sharing the arm is safe because the attribute-extraction code below
                // is correct for both, and no real Empty event matches a text_target tag.
                match e.name().as_ref() {
                    b"Title" if !in_library_descriptor => {
                        text_buf.clear();
                        text_target = Some(TextTarget::ExperimentTitle);
                    }
                    b"Platform" => {
                        data.platform.name = None;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"instrument_model" {
                                let v = attr.unescape_value()
                                    .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?
                                    .into_owned();
                                data.platform.instrument_model = Some(v);
                            }
                        }
                        text_buf.clear();
                        text_target = Some(TextTarget::PlatformName);
                    }
                    b"Statistics" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"total_runs" => data.total_runs = val.parse().ok(),
                                b"total_spots" => data.total_spots = val.parse().ok(),
                                b"total_bases" => data.total_bases = val.parse().ok(),
                                b"total_size" => data.total_size = val.parse().ok(),
                                _ => {}
                            }
                        }
                    }
                    b"Experiment" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"acc" => data.experiment_accession = val.into_owned(),
                                b"status" => data.experiment_status = Some(val.into_owned()),
                                b"name" => {
                                    if data.experiment_title.is_none() {
                                        data.experiment_title = Some(val.into_owned());
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"Study" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"acc" => data.study_accession = val.into_owned(),
                                b"name" => data.study_title = Some(val.into_owned()),
                                _ => {}
                            }
                        }
                    }
                    b"Sample" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"acc" => data.sample_accession = val.into_owned(),
                                b"name" => {
                                    let s = val.into_owned();
                                    if !s.is_empty() {
                                        data.sample_name = Some(s);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    b"Organism" => {
                        for attr in e.attributes().flatten() {
                            let val = attr.unescape_value()
                                .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                            match attr.key.as_ref() {
                                b"taxid" => data.organism_taxid = val.parse().ok(),
                                b"ScientificName" => data.organism_name = Some(val.into_owned()),
                                _ => {}
                            }
                        }
                    }
                    b"Bioproject" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::Bioproject);
                    }
                    b"Biosample" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::Biosample);
                    }
                    b"Library_descriptor" => in_library_descriptor = true,
                    b"LIBRARY_STRATEGY" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibStrategy);
                    }
                    b"LIBRARY_SOURCE" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibSource);
                    }
                    b"LIBRARY_SELECTION" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibSelection);
                    }
                    b"LIBRARY_LAYOUT" => in_library_layout = true,
                    b"PAIRED" if in_library_layout => {
                        data.library.layout = Some(LibraryLayout::Paired { nominal_length: None, nominal_sdev: None });
                    }
                    b"SINGLE" if in_library_layout => {
                        data.library.layout = Some(LibraryLayout::Single { length: None });
                    }
                    b"LIBRARY_CONSTRUCTION_PROTOCOL" => {
                        text_buf.clear();
                        text_target = Some(TextTarget::LibProtocol);
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if text_target.is_some() {
                    let s = e.unescape().map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                    text_buf.push_str(&s);
                }
            }
            Ok(Event::End(e)) => {
                match e.name().as_ref() {
                    b"Library_descriptor" => in_library_descriptor = false,
                    b"LIBRARY_LAYOUT" => in_library_layout = false,
                    _ => {}
                }
                if let Some(target) = text_target.take() {
                    let value = std::mem::take(&mut text_buf);
                    let value = value.trim().to_owned();
                    let value_opt = if value.is_empty() { None } else { Some(value) };
                    match target {
                        TextTarget::ExperimentTitle => data.experiment_title = value_opt,
                        TextTarget::PlatformName => data.platform.name = value_opt,
                        TextTarget::Bioproject => data.bioproject = value_opt,
                        TextTarget::Biosample => data.biosample = value_opt,
                        TextTarget::LibStrategy => data.library.strategy = value_opt,
                        TextTarget::LibSource => data.library.source = value_opt,
                        TextTarget::LibSelection => data.library.selection = value_opt,
                        TextTarget::LibProtocol => data.library.construction_protocol = value_opt,
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(data)
}

/// Project an `ExpXmlData` into the public `Experiment`, `Study`, `Sample` types.
#[must_use]
pub fn project(data: ExpXmlData) -> (Experiment, Study, Sample) {
    let study = Study {
        accession: data.study_accession.clone(),
        title: data.study_title,
        abstract_: None,
        bioproject: data.bioproject,
        geo_accession: None,
        pmids: vec![],
    };
    let experiment = Experiment {
        accession: data.experiment_accession.clone(),
        title: data.experiment_title,
        study_accession: data.study_accession.clone(),
        sample_accession: data.sample_accession.clone(),
        design_description: None,
        library: data.library,
        platform: data.platform,
        geo_accession: None,
    };
    let sample = Sample {
        accession: data.sample_accession,
        title: data.sample_name,
        biosample: data.biosample,
        organism_taxid: data.organism_taxid,
        organism_name: data.organism_name,
        attributes: Default::default(),
    };
    (experiment, study, sample)
}

#[derive(Debug, Clone, Copy)]
enum TextTarget {
    ExperimentTitle,
    PlatformName,
    Bioproject,
    Biosample,
    LibStrategy,
    LibSource,
    LibSelection,
    LibProtocol,
}

/// One run in the Runs fragment.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawRun {
    pub accession: String,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub is_public: Option<bool>,
}

/// Parse the decoded Runs fragment into a list of raw runs.
pub fn parse_runs(fragment: &str) -> Result<Vec<RawRun>> {
    let wrapped = format!("<Root>{fragment}</Root>");
    let mut reader = Reader::from_str(&wrapped);
    reader.config_mut().trim_text(true);

    let mut runs: Vec<RawRun> = Vec::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(SradbError::Xml { context: CONTEXT, source: e }),
            Ok(Event::Eof) => break,
            Ok(Event::Empty(e)) | Ok(Event::Start(e)) => {
                if e.name().as_ref() == b"Run" {
                    let mut r = RawRun::default();
                    for attr in e.attributes().flatten() {
                        let val = attr.unescape_value()
                            .map_err(|e| SradbError::Xml { context: CONTEXT, source: e })?;
                        match attr.key.as_ref() {
                            b"acc" => r.accession = val.into_owned(),
                            b"total_spots" => r.total_spots = val.parse().ok(),
                            b"total_bases" => r.total_bases = val.parse().ok(),
                            b"is_public" => r.is_public = match val.as_ref() {
                                "true" => Some(true),
                                "false" => Some(false),
                                _ => None,
                            },
                            _ => {}
                        }
                    }
                    runs.push(r);
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(runs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAGMENT: &str = r#"<Summary><Title>GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq</Title><Platform instrument_model="Illumina HiSeq 2000">ILLUMINA</Platform><Statistics total_runs="1" total_spots="38671668" total_bases="11678843736" total_size="5132266976" load_done="true" cluster_name="public"/></Summary><Submitter acc="SRA826111" center_name="GEO"/><Experiment acc="SRX5172107" ver="1" status="public" name="GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq"/><Study acc="SRP174132" name="ARID1A is a critical regulator of luminal identity and therapeutic response in oestrogen receptor-positive breast cancer (RNA-Seq)"/><Organism taxid="9606" ScientificName="Homo sapiens"/><Sample acc="SRS4179725" name=""/><Instrument ILLUMINA="Illumina HiSeq 2000"/><Library_descriptor><LIBRARY_STRATEGY>RNA-Seq</LIBRARY_STRATEGY><LIBRARY_SOURCE>TRANSCRIPTOMIC</LIBRARY_SOURCE><LIBRARY_SELECTION>cDNA</LIBRARY_SELECTION><LIBRARY_LAYOUT><PAIRED/></LIBRARY_LAYOUT><LIBRARY_CONSTRUCTION_PROTOCOL>RNA was isolated using the Qiagen RNeasy kit.</LIBRARY_CONSTRUCTION_PROTOCOL></Library_descriptor><Bioproject>PRJNA511021</Bioproject><Biosample>SAMN10621858</Biosample>"#;

    #[test]
    fn parses_full_fragment() {
        let data = parse(FRAGMENT).unwrap();
        assert_eq!(data.experiment_accession, "SRX5172107");
        assert_eq!(data.study_accession, "SRP174132");
        assert_eq!(data.sample_accession, "SRS4179725");
        assert_eq!(data.organism_taxid, Some(9606));
        assert_eq!(data.organism_name.as_deref(), Some("Homo sapiens"));
        assert_eq!(data.bioproject.as_deref(), Some("PRJNA511021"));
        assert_eq!(data.biosample.as_deref(), Some("SAMN10621858"));
        assert_eq!(data.platform.name.as_deref(), Some("ILLUMINA"));
        assert_eq!(data.platform.instrument_model.as_deref(), Some("Illumina HiSeq 2000"));
        assert_eq!(data.library.strategy.as_deref(), Some("RNA-Seq"));
        assert_eq!(data.library.source.as_deref(), Some("TRANSCRIPTOMIC"));
        assert_eq!(data.library.selection.as_deref(), Some("cDNA"));
        assert!(matches!(data.library.layout, Some(LibraryLayout::Paired { .. })));
        assert_eq!(data.total_spots, Some(38_671_668));
        assert_eq!(data.total_bases, Some(11_678_843_736));
        assert_eq!(data.total_size, Some(5_132_266_976));
        assert_eq!(data.total_runs, Some(1));
    }

    #[test]
    fn project_into_public_types() {
        let data = parse(FRAGMENT).unwrap();
        let (exp, study, sample) = project(data);
        assert_eq!(exp.accession, "SRX5172107");
        assert_eq!(exp.study_accession, "SRP174132");
        assert_eq!(exp.sample_accession, "SRS4179725");
        assert_eq!(study.accession, "SRP174132");
        assert!(study.title.unwrap().starts_with("ARID1A is a critical regulator"));
        assert_eq!(sample.accession, "SRS4179725");
        assert_eq!(sample.organism_name.as_deref(), Some("Homo sapiens"));
    }

    #[test]
    fn parses_runs_single() {
        let frag = r#"<Run acc="SRR8361601" total_spots="38671668" total_bases="11678843736" load_done="true" is_public="true" cluster_name="public" static_data_available="true"/>"#;
        let runs = parse_runs(frag).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].accession, "SRR8361601");
        assert_eq!(runs[0].total_spots, Some(38_671_668));
        assert_eq!(runs[0].total_bases, Some(11_678_843_736));
        assert_eq!(runs[0].is_public, Some(true));
    }

    #[test]
    fn parses_runs_multiple() {
        let frag = r#"<Run acc="SRR1" total_spots="100"/><Run acc="SRR2" total_spots="200"/>"#;
        let runs = parse_runs(frag).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].accession, "SRR1");
        assert_eq!(runs[1].accession, "SRR2");
        assert_eq!(runs[1].total_spots, Some(200));
    }
}
