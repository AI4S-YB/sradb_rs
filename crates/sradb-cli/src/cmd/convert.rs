//! `sradb convert <FROM> <TO> <ACCESSION>...` handler.

use clap::Args;
use sradb_core::accession::{Accession, AccessionKind};
use sradb_core::{ClientConfig, SraClient};

/// CLI-friendly value for AccessionKind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum CliAccKind {
    Srp,
    Srx,
    Srr,
    Srs,
    Gse,
    Gsm,
}

impl From<CliAccKind> for AccessionKind {
    fn from(c: CliAccKind) -> Self {
        match c {
            CliAccKind::Srp => AccessionKind::Srp,
            CliAccKind::Srx => AccessionKind::Srx,
            CliAccKind::Srr => AccessionKind::Srr,
            CliAccKind::Srs => AccessionKind::Srs,
            CliAccKind::Gse => AccessionKind::Gse,
            CliAccKind::Gsm => AccessionKind::Gsm,
        }
    }
}

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// Source accession kind.
    #[arg(value_enum)]
    pub from: CliAccKind,

    /// Target accession kind.
    #[arg(value_enum)]
    pub to: CliAccKind,

    /// One or more accessions of the source kind.
    #[arg(required = true)]
    pub accessions: Vec<String>,
}

pub async fn run(args: ConvertArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let to_kind: AccessionKind = args.to.into();
    let expected_from: AccessionKind = args.from.into();

    let mut had_error = false;
    for raw in &args.accessions {
        let input: Accession = match raw.parse() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("error parsing {raw}: {e}");
                had_error = true;
                continue;
            }
        };
        if input.kind != expected_from {
            eprintln!(
                "error: {raw} parses as {:?}, but --from said {:?}",
                input.kind, expected_from,
            );
            had_error = true;
            continue;
        }
        match client.convert(&input, to_kind).await {
            Ok(results) => {
                if results.is_empty() {
                    eprintln!("warning: no results for {raw}");
                }
                for r in &results {
                    println!("{}\t{}", input.raw, r.raw);
                }
            }
            Err(e) => {
                eprintln!("error converting {raw}: {e}");
                had_error = true;
            }
        }
    }
    if had_error {
        std::process::exit(1);
    }
    Ok(())
}
