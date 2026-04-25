use proptest::prelude::*;
use sradb_core::accession::{Accession, AccessionKind};

fn accession_strategy() -> impl Strategy<Value = (AccessionKind, String)> {
    prop_oneof![
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srp, format!("SRP{n:06}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srx, format!("SRX{n:06}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srs, format!("SRS{n:06}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Srr, format!("SRR{n:06}"))),
        (1u32..=999_999u32).prop_map(|n| (AccessionKind::Gse, format!("GSE{n}"))),
        (1u32..=9_999_999u32).prop_map(|n| (AccessionKind::Gsm, format!("GSM{n}"))),
        (1u32..=99_999_999u32).prop_map(|n| (AccessionKind::Pmid, format!("{n}"))),
        (1u32..=99_999_999u32).prop_map(|n| (AccessionKind::Pmc, format!("PMC{n}"))),
    ]
}

proptest! {
    #[test]
    fn parse_and_display_round_trip((expected_kind, raw) in accession_strategy()) {
        let acc: Accession = raw.parse().unwrap();
        prop_assert_eq!(acc.kind, expected_kind);
        prop_assert_eq!(acc.to_string(), raw.clone());
        prop_assert_eq!(acc.raw, raw);
    }
}
