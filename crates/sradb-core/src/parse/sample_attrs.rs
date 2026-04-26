//! Parser for pysradb-style serialized `sample_attribute` strings.
//!
//! Format: `key: value || key: value || key: value`. Values may contain
//! colons (`source_name: Liver: Adult`) — only the FIRST `:` separates key/value.

use std::collections::BTreeMap;

/// Parse a pipe-delimited `key: value` string.
/// Whitespace around keys and values is trimmed. Empty entries are dropped.
#[must_use]
pub fn parse(input: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for entry in input.split("||") {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((k, v)) = entry.split_once(':') {
            let key = k.trim().to_owned();
            let val = v.trim().to_owned();
            if !key.is_empty() {
                out.insert(key, val);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple() {
        let m = parse("source_name: liver || cell type: hepatocyte");
        assert_eq!(m.get("source_name").map(String::as_str), Some("liver"));
        assert_eq!(m.get("cell type").map(String::as_str), Some("hepatocyte"));
    }

    #[test]
    fn value_with_colon() {
        let m = parse("source_name: Liver: Adult");
        assert_eq!(
            m.get("source_name").map(String::as_str),
            Some("Liver: Adult")
        );
    }

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(parse("").is_empty());
        assert!(parse("   ").is_empty());
        assert!(parse(" || ").is_empty());
    }

    #[test]
    fn trims_whitespace() {
        let m = parse("  k1  :  v1  ||  k2:v2 ");
        assert_eq!(m.get("k1").map(String::as_str), Some("v1"));
        assert_eq!(m.get("k2").map(String::as_str), Some("v2"));
    }
}
