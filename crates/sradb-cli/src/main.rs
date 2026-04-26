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
    /// Search SRA with field-qualified Entrez queries.
    Search(cmd::search::SearchArgs),
    /// Download SRA / ENA fastq files for accessions.
    Download(cmd::download::DownloadArgs),
    /// GEO helpers (matrix download/parse).
    Geo(cmd::geo::GeoArgs),
    /// Extract database identifiers from PMID / DOI / PMC.
    Id(cmd::id::IdArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
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
