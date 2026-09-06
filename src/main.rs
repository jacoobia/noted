use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
    /// Handle a Claude Code `PreToolUse` hook event from stdin
    Hook,
}

fn main() -> ExitCode {
    let _cli = Cli::parse();
    eprintln!("not implemented");
    ExitCode::FAILURE
}
