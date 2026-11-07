use std::io::Read;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::{ArgGroup, Parser, Subcommand};
use noted::cache::{Store, db_path};
use noted::get::{self, GetArgs};
use noted::{config, manage};

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
        #[arg(long, value_parser = non_blank)]
        section: Option<String>,
        /// Skip the status line on stderr
        #[arg(long, short)]
        quiet: bool,
    },
    /// List cached entries
    Ls,
    /// Remove cached entries
    #[command(group(ArgGroup::new("target").required(true).args(["urls", "stale", "all"])))]
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
    /// Show or change settings
    Config {
        #[command(subcommand)]
        setting: Setting,
    },
    /// Handle a Claude Code pre-tool-use hook event from stdin
    Hook,
}

/// Claude Code treats exit code 2 from a hook as "block this tool call", so usage errors
/// use 64 (`EX_USAGE`) rather than clap's default of 2.
const USAGE_ERROR: u8 = 64;

#[derive(Subcommand)]
enum Setting {
    /// Print the User-Agent requests are sent with, or save a new one
    UserAgent {
        #[arg(value_parser = config::validate_user_agent)]
        value: Option<String>,
    },
}

fn main() -> ExitCode {
    let command = match Cli::try_parse() {
        Ok(cli) => cli.command,
        Err(err) => {
            let _ = err.print();
            return if err.use_stderr() {
                ExitCode::from(USAGE_ERROR)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    if matches!(command, Command::Hook) {
        hook();
        return ExitCode::SUCCESS;
    }
    run(command).unwrap_or_else(|err| {
        eprintln!("noted: {err:#}");
        if err.is::<config::SetupNeeded>() {
            ExitCode::from(config::SETUP_NEEDED)
        } else if err.is::<config::InvalidEnvUserAgent>() {
            ExitCode::from(USAGE_ERROR)
        } else {
            ExitCode::FAILURE
        }
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
            get::cache_key(&url)?;
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
        Command::Ls => manage::ls(&Store::open(&db_path())?).map(|()| ExitCode::SUCCESS),
        Command::Rm { urls, stale, all } => {
            manage::rm(&Store::open(&db_path())?, &urls, stale, all).map(|()| ExitCode::SUCCESS)
        }
        Command::Stats => {
            let path = db_path();
            manage::stats(&Store::open(&path)?, &path).map(|()| ExitCode::SUCCESS)
        }
        Command::Config {
            setting: Setting::UserAgent { value },
        } => user_agent_setting(value),
        Command::Hook => unreachable!("handled in main"),
    }
}

fn user_agent_setting(value: Option<String>) -> Result<ExitCode> {
    if let Some(value) = value {
        let path = config::save_user_agent(&value)?;
        eprintln!("noted: saved to {}", path.display());
        return Ok(ExitCode::SUCCESS);
    }
    let Some((current, _)) = config::user_agent()? else {
        eprintln!("noted: not set. Save one with: noted config user-agent \"<value>\"");
        return Ok(ExitCode::from(config::SETUP_NEEDED));
    };
    noted::output::emit(format!("{current}\n").as_bytes())?;
    Ok(ExitCode::SUCCESS)
}

fn hook() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let allow_webfetch = std::env::var("NOTED_ALLOW_WEBFETCH").is_ok_and(|v| v == "1");
    if let Some(response) = noted::hook::respond(&input, allow_webfetch) {
        let _ = noted::output::emit(response.as_bytes());
    }
}

fn non_blank(value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err("must not be blank".to_owned())
    } else {
        Ok(trimmed.to_owned())
    }
}
