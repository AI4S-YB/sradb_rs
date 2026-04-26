//! `sradb id <PMID|DOI|PMC>` handler.

use clap::Args;
use sradb_core::{ClientConfig, SraClient};

#[derive(Args, Debug)]
pub struct IdArgs {
    /// One identifier (PMID number, PMC accession like `PMC10802650`, or DOI like `10.1234/abcd`).
    pub identifier: String,

    /// Output as JSON instead of plaintext.
    #[arg(long, default_value_t = false)]
    pub json: bool,
}

pub async fn run(args: IdArgs) -> anyhow::Result<()> {
    let cfg = ClientConfig::default();
    let client = SraClient::with_config(cfg)?;
    let id = args.identifier.trim();

    let set = if id.starts_with("PMC") {
        client.identifiers_from_pmc(id).await?
    } else if id.starts_with("10.") {
        client.identifiers_from_doi(id).await?
    } else if let Ok(pmid) = id.parse::<u64>() {
        client.identifiers_from_pmid(pmid).await?
    } else {
        return Err(anyhow::anyhow!(
            "unrecognized identifier: {id} (expected PMID number, PMC<digits>, or DOI starting with 10.)"
        ));
    };

    if args.json {
        println!("{}", serde_json::to_string_pretty(&set)?);
    } else {
        if let Some(p) = set.pmid {
            println!("pmid:\t{p}");
        }
        if let Some(p) = &set.pmc_id {
            println!("pmc:\t{p}");
        }
        if let Some(d) = &set.doi {
            println!("doi:\t{d}");
        }
        for g in &set.gse_ids {
            println!("gse:\t{g}");
        }
        for g in &set.gsm_ids {
            println!("gsm:\t{g}");
        }
        for s in &set.srp_ids {
            println!("srp:\t{s}");
        }
        for p in &set.prjna_ids {
            println!("prjna:\t{p}");
        }
    }
    Ok(())
}
