//! Public typed structs returned by the metadata API.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryLayout {
    Single {
        length: Option<u32>,
    },
    Paired {
        nominal_length: Option<u32>,
        nominal_sdev: Option<f32_serde::F32Eq>,
    },
    Unknown,
}

#[allow(non_snake_case)]
mod f32_serde {
    use serde::{Deserialize, Serialize};

    /// `f32` wrapper that derives `Eq` (because we use `Option<f32>`-ish in `PartialEq` tests
    /// and want to keep `LibraryLayout: Eq`). Equality is bitwise on the `to_bits` representation.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct F32Eq(pub f32);

    impl PartialEq for F32Eq {
        fn eq(&self, other: &Self) -> bool {
            self.0.to_bits() == other.0.to_bits()
        }
    }
    impl Eq for F32Eq {}
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Library {
    pub strategy: Option<String>,
    pub source: Option<String>,
    pub selection: Option<String>,
    pub layout: Option<LibraryLayout>,
    pub construction_protocol: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Platform {
    pub name: Option<String>,
    pub instrument_model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Study {
    pub accession: String,
    pub title: Option<String>,
    pub abstract_: Option<String>,
    pub bioproject: Option<String>,
    pub geo_accession: Option<String>,
    pub pmids: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Experiment {
    pub accession: String,
    pub title: Option<String>,
    pub study_accession: String,
    pub sample_accession: String,
    pub design_description: Option<String>,
    pub library: Library,
    pub platform: Platform,
    pub geo_accession: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Sample {
    pub accession: String,
    pub title: Option<String>,
    pub biosample: Option<String>,
    pub organism_taxid: Option<u32>,
    pub organism_name: Option<String>,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RunUrls {
    pub ena_fastq_http: Vec<String>,
    pub ena_fastq_ftp: Vec<String>,
    pub ncbi_sra: Option<String>,
    pub s3: Option<String>,
    pub gs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Run {
    pub accession: String,
    pub experiment_accession: String,
    pub sample_accession: String,
    pub study_accession: String,
    pub total_spots: Option<u64>,
    pub total_bases: Option<u64>,
    pub total_size: Option<u64>,
    pub published: Option<String>,
    pub urls: RunUrls,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Enrichment {
    pub organ: Option<String>,
    pub tissue: Option<String>,
    pub anatomical_system: Option<String>,
    pub cell_type: Option<String>,
    pub disease: Option<String>,
    pub sex: Option<String>,
    pub development_stage: Option<String>,
    pub assay: Option<String>,
    pub organism: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataRow {
    pub run: Run,
    pub experiment: Experiment,
    pub sample: Sample,
    pub study: Study,
    pub enrichment: Option<Enrichment>,
}

#[derive(Debug, Clone, Default)]
pub struct MetadataOpts {
    /// Slice 3 enables this. Slice 2 ignores it (always defaults to false).
    pub detailed: bool,
    /// Slice 7 enables this.
    pub enrich: bool,
    /// Pagination page size for esummary calls.
    pub page_size: u32,
}

impl MetadataOpts {
    #[must_use]
    pub fn new() -> Self {
        Self {
            detailed: false,
            enrich: false,
            page_size: 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_row_round_trips_through_json() {
        let row = MetadataRow {
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
                title: Some(
                    "GSM3526037: RNA-Seq Sample_DMSO_sg6_KO_2; Homo sapiens; RNA-Seq".into(),
                ),
                study_accession: "SRP174132".into(),
                sample_accession: "SRS4179725".into(),
                design_description: None,
                library: Library {
                    strategy: Some("RNA-Seq".into()),
                    source: Some("TRANSCRIPTOMIC".into()),
                    selection: Some("cDNA".into()),
                    layout: Some(LibraryLayout::Paired {
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
                title: Some("ARID1A is a critical regulator of luminal identity ...".into()),
                abstract_: None,
                bioproject: Some("PRJNA511021".into()),
                geo_accession: None,
                pmids: vec![],
            },
            enrichment: None,
        };

        let json = serde_json::to_string(&row).unwrap();
        let back: MetadataRow = serde_json::from_str(&json).unwrap();
        assert_eq!(row, back);
    }
}
