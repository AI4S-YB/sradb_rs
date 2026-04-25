//! sradb command-line interface.

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
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Some(Cmd::Info) => {
            println!("sradb {}", env!("CARGO_PKG_VERSION"));
            println!("https://github.com/saketkc/pysradb (Rust port)");
            Ok(())
        }
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
