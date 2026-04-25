//! Parser for the outer `<eSummaryResult>` envelope returned by NCBI esummary.
//!
//! Each `<DocSum>` carries five `<Item>` children. Slice 2 needs `ExpXml` and
//! `Runs` (both XML-encoded XML fragments — kept as raw strings here and
//! decoded by `parse::exp_xml` in Task 4).

use quick_xml::events::Event;
use quick_xml::reader::Reader;

use crate::error::{Result, SradbError};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RawDocSum {
    pub id: String,
    pub exp_xml: String,
    pub runs: String,
    pub create_date: Option<String>,
    pub update_date: Option<String>,
}

const CONTEXT: &str = "esummary";

/// Parse the `<eSummaryResult>` body into a list of raw doc-sums.
///
/// `body` is the full XML response body (including the XML preamble).
pub fn parse(body: &str) -> Result<Vec<RawDocSum>> {
    let mut reader = Reader::from_str(body);
    reader.config_mut().trim_text(true);

    let mut docs: Vec<RawDocSum> = Vec::new();
    let mut current: Option<RawDocSum> = None;
    let mut current_item_name: Option<String> = None;
    let mut current_text = String::new();
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return Err(SradbError::Xml {
                    context: CONTEXT,
                    source: e,
                });
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"DocSum" => {
                        current = Some(RawDocSum::default());
                    }
                    b"Item" => {
                        let mut item_name: Option<String> = None;
                        for attr in e.attributes().flatten() {
                            if attr.key.as_ref() == b"Name" {
                                item_name = Some(
                                    attr.unescape_value()
                                        .map_err(|e| SradbError::Xml {
                                            context: CONTEXT,
                                            source: e,
                                        })?
                                        .into_owned(),
                                );
                            }
                        }
                        current_item_name = item_name;
                        current_text.clear();
                    }
                    b"Id" => {
                        current_text.clear();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                let text = e.unescape().map_err(|e| SradbError::Xml {
                    context: CONTEXT,
                    source: e,
                })?;
                current_text.push_str(&text);
            }
            Ok(Event::CData(e)) => {
                let text = std::str::from_utf8(e.as_ref()).map_err(|err| SradbError::Parse {
                    endpoint: CONTEXT,
                    message: format!("CDATA not utf-8: {err}"),
                })?;
                current_text.push_str(text);
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                match name.as_ref() {
                    b"DocSum" => {
                        if let Some(d) = current.take() {
                            docs.push(d);
                        }
                    }
                    b"Item" => {
                        if let (Some(item_name), Some(d)) =
                            (current_item_name.take(), current.as_mut())
                        {
                            match item_name.as_str() {
                                "ExpXml" => d.exp_xml = std::mem::take(&mut current_text),
                                "Runs" => d.runs = std::mem::take(&mut current_text),
                                "CreateDate" => {
                                    d.create_date = Some(std::mem::take(&mut current_text));
                                }
                                "UpdateDate" => {
                                    d.update_date = Some(std::mem::take(&mut current_text));
                                }
                                _ => current_text.clear(),
                            }
                        }
                    }
                    b"Id" => {
                        if let Some(d) = current.as_mut() {
                            d.id = std::mem::take(&mut current_text);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(docs)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0"?>
<eSummaryResult>
  <DocSum>
    <Id>123</Id>
    <Item Name="ExpXml" Type="String">&lt;Summary&gt;&lt;Title&gt;hi&lt;/Title&gt;&lt;/Summary&gt;</Item>
    <Item Name="Runs" Type="String">&lt;Run acc="SRR1"/&gt;</Item>
    <Item Name="ExtLinks" Type="String"></Item>
    <Item Name="CreateDate" Type="String">2024/01/02</Item>
    <Item Name="UpdateDate" Type="String">2024/02/03</Item>
  </DocSum>
</eSummaryResult>"#;

    #[test]
    fn parses_one_docsum() {
        let docs = parse(SAMPLE).unwrap();
        assert_eq!(docs.len(), 1);
        let d = &docs[0];
        assert_eq!(d.id, "123");
        assert!(
            d.exp_xml.contains("<Summary>"),
            "exp_xml decoded: {}",
            d.exp_xml
        );
        assert!(d.exp_xml.contains("<Title>hi</Title>"));
        assert!(d.runs.contains(r#"<Run acc="SRR1"/>"#));
        assert_eq!(d.create_date.as_deref(), Some("2024/01/02"));
        assert_eq!(d.update_date.as_deref(), Some("2024/02/03"));
    }

    #[test]
    fn parses_real_srp174132_fixture() {
        let body = std::fs::read_to_string(
            sradb_fixtures::workspace_root().join("tests/data/ncbi/esummary_SRP174132.xml"),
        )
        .expect("run `cargo run -p capture-fixtures -- save-esummary SRP174132` first");
        let docs = parse(&body).unwrap();
        assert!(!docs.is_empty(), "should have at least 1 docsum");
        for d in &docs {
            assert!(!d.id.is_empty());
            assert!(
                d.exp_xml.contains("<Study"),
                "ExpXml should contain <Study>; got: {}",
                &d.exp_xml[..d.exp_xml.len().min(200)]
            );
            assert!(d.runs.contains("<Run "), "Runs should contain <Run>");
        }
    }
}
