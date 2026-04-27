//! sradb command-line interface.

mod cmd;
mod output;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "sradb", version, about = "Query NGS metadata from SRA / ENA / GEO.", long_about = None)]
struct Cli {
    /// Increase verbosity (-v info, -vv debug, -vvv trace).
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Print build information and exit.
    Info,
    /// Fetch metadata for one or more accessions.
    Metadata(cmd::metadata::MetadataArgs),
    /// Convert accessions between SRA / GEO kinds (e.g. `srp srx SRP174132`).
    Convert(cmd::convert::ConvertArgs),
    /// Search SRA / ENA / GEO with field-qualified queries.
    Search(cmd::search::SearchArgs),
    /// Download SRA / ENA fastq files for accessions.
    Download(cmd::download::DownloadArgs),
    /// GEO helpers (matrix download/parse).
    Geo(cmd::geo::GeoArgs),
    /// Extract database identifiers from PMID / DOI / PMC.
    Id(cmd::id::IdArgs),
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let dispatch = run_command(cli.command);
    let result = tokio::select! {
        res = dispatch => res,
        _ = tokio::signal::ctrl_c() => {
            eprintln!("\ninterrupted (Ctrl-C); partial .part files preserved for resume");
            std::process::exit(130);
        }
    };

    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(classify_error(&err));
    }
}

async fn run_command(command: Option<Cmd>) -> anyhow::Result<()> {
    match command {
        Some(Cmd::Info) => {
            println!("sradb {}", env!("CARGO_PKG_VERSION"));
            println!("https://github.com/saketkc/pysradb (Rust port)");
            Ok(())
        }
        Some(Cmd::Metadata(args)) => cmd::metadata::run(args).await,
        Some(Cmd::Convert(args)) => cmd::convert::run(args).await,
        Some(Cmd::Search(args)) => cmd::search::run(args).await,
        Some(Cmd::Download(args)) => cmd::download::run(args).await,
        Some(Cmd::Geo(args)) => cmd::geo::run(args).await,
        Some(Cmd::Id(args)) => cmd::id::run(args).await,
        None => {
            <Cli as clap::CommandFactory>::command().print_help()?;
            println!();
            Ok(())
        }
    }
}

/// Map an error to a CLI exit code per the spec:
/// - `3` enrichment auth/missing-key failures
/// - `4` checksum verification failed
/// - `1` everything else
fn classify_error(err: &anyhow::Error) -> i32 {
    use sradb_core::SradbError;
    if let Some(sradb) = err.downcast_ref::<SradbError>() {
        return match sradb {
            SradbError::ChecksumMismatch { .. } => 4,
            SradbError::Enrichment { message, .. } if is_auth_failure(message) => 3,
            _ => 1,
        };
    }
    1
}

fn is_auth_failure(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("openai_api_key")
        || m.contains("status 401")
        || m.contains("status 403")
        || m.contains("invalid api key")
        || m.contains("authentication")
}

fn init_tracing(verbosity: u8) {
    use tracing_subscriber::{fmt, EnvFilter};

    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(format!(
            "sradb={level},sradb_core={level},sradb_cli={level}"
        ))
    });
    fmt().with_env_filter(filter).with_target(false).init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use sradb_core::SradbError;

    #[test]
    fn classify_checksum_mismatch_is_four() {
        let err: anyhow::Error = SradbError::ChecksumMismatch {
            path: "/tmp/x.sra".into(),
            expected: "abc".into(),
            got: "xyz".into(),
        }
        .into();
        assert_eq!(classify_error(&err), 4);
    }

    #[test]
    fn classify_missing_openai_key_is_three() {
        let err: anyhow::Error = SradbError::Enrichment {
            message: "OPENAI_API_KEY not set; cannot enrich".into(),
            source: None,
        }
        .into();
        assert_eq!(classify_error(&err), 3);
    }

    #[test]
    fn classify_openai_401_is_three() {
        let err: anyhow::Error = SradbError::Enrichment {
            message: "status 401 Unauthorized from https://api.openai.com/...".into(),
            source: None,
        }
        .into();
        assert_eq!(classify_error(&err), 3);
    }

    #[test]
    fn classify_other_enrichment_is_one() {
        let err: anyhow::Error = SradbError::Enrichment {
            message: "status 500 from https://api.openai.com/...".into(),
            source: None,
        }
        .into();
        assert_eq!(classify_error(&err), 1);
    }

    #[test]
    fn classify_unrelated_error_is_one() {
        let err: anyhow::Error = SradbError::NotFound("SRP1".into()).into();
        assert_eq!(classify_error(&err), 1);
    }

    #[test]
    fn classify_anyhow_only_error_is_one() {
        let err = anyhow::anyhow!("clap parse failure");
        assert_eq!(classify_error(&err), 1);
    }
}
