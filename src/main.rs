use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use noted::cache::{Store, db_path};
use noted::get::{self, GetArgs};

#[derive(Parser)]
#[command(version, about = "Fetch URLs through a local cache")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch a URL, serving it from the cache while fresh
    Get {
        url: String,
        /// How long the copy stays fresh, e.g. 30m, 7d (default 1d, or the stored TTL)
        #[arg(long, value_parser = humantime::parse_duration)]
        ttl: Option<Duration>,
        /// Fetch again even if the cached copy is fresh
        #[arg(long)]
        refresh: bool,
        /// Print HTML as-is instead of converting it to markdown
        #[arg(long)]
        raw: bool,
        /// Print only the section under this heading
        #[arg(long)]
        section: Option<String>,
        /// Skip the status line on stderr
        #[arg(long, short)]
        quiet: bool,
    },
    /// List cached entries
    Ls,
    /// Remove cached entries
    Rm {
        urls: Vec<String>,
        /// Remove every stale entry
        #[arg(long)]
        stale: bool,
        /// Remove everything
        #[arg(long)]
        all: bool,
    },
    /// Show cache totals
    Stats,
    /// Handle a Claude Code pre-tool-use hook event from stdin
    Hook,
}

fn main() -> ExitCode {
    run(Cli::parse().command).unwrap_or_else(|err| {
        eprintln!("noted: {err:#}");
        ExitCode::FAILURE
    })
}

fn run(command: Command) -> Result<ExitCode> {
    match command {
        Command::Get {
            url,
            ttl,
            refresh,
            raw,
            section,
            quiet,
        } => {
            let store = Store::open(&db_path())?;
            get::run(
                &store,
                &GetArgs {
                    url,
                    ttl,
                    refresh,
                    raw,
                    section,
                    quiet,
                },
            )
        }
        Command::Ls | Command::Rm { .. } | Command::Stats | Command::Hook => {
            bail!("not implemented")
        }
    }
}
