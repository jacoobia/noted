use std::fmt::Write;
use std::io::Read;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Result;
use clap::{ArgGroup, Parser, Subcommand};
use noted::cache::{Store, db_path};
use noted::get::{self, Format, GetArgs, Select};
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
    #[command(long_about = GET_HELP)]
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
        /// Print link text without the link targets, to save tokens
        #[arg(long, conflicts_with = "raw")]
        plain: bool,
        /// Print only the section under this heading
        #[arg(long, value_parser = non_blank)]
        section: Option<String>,
        /// Print only the page's headings
        #[arg(long, conflicts_with = "section")]
        outline: bool,
        /// Longest to wait for a rate-limited host before giving up with exit code 4
        #[arg(long, value_parser = humantime::parse_duration, default_value = "90s")]
        max_wait: Duration,
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
    #[command(long_about = HOOK_HELP)]
    Hook,
}

/// Claude Code treats exit code 2 from a hook as "block this tool call", so usage errors
/// use 64 (`EX_USAGE`) rather than clap's default of 2.
const USAGE_ERROR: u8 = 64;

const GET_HELP: &str = "Fetch a URL, serving it from the cache while fresh.

A copy stays fresh for its TTL, which counts from the last time the server sent it or confirmed it unchanged (a 304). Once it goes stale, noted asks the server again. If the server can't be reached (offline, a timeout or a 5xx), you get the stale copy with a warning. A page that was never cached can't be fetched offline.

Exit codes: 0 success, 1 an error or a robots.txt refusal, 3 no User-Agent set yet, 4 the host is rate limited for longer than --max-wait, 64 bad arguments.";

const HOOK_HELP: &str = "Handle a Claude Code pre-tool-use hook event from stdin.

Reads one PreToolUse event as JSON. A plain `curl <url>` Bash call becomes `noted get <url>`, \
and a piped `curl <url> | ...` becomes `noted get --raw <url> | ...`. WebFetch calls are \
denied with a message pointing at noted; set NOTED_ALLOW_WEBFETCH=1 to allow them. Anything \
else passes through untouched. It always exits 0, so it can never block a tool call.

To install, add this to ~/.claude/settings.json. The permission rule lets rewritten commands \
run without a prompt:

{
  \"hooks\": {
    \"PreToolUse\": [
      { \"matcher\": \"Bash|WebFetch\", \"hooks\": [{ \"type\": \"command\", \"command\": \"noted hook\" }] }
    ]
  },
  \"permissions\": { \"allow\": [\"Bash(noted get:*)\"] }
}";

#[derive(Subcommand)]
enum Setting {
    /// Print the User-Agent requests are sent with, or save a new one
    UserAgent {
        #[arg(value_parser = config::validate_user_agent)]
        value: Option<String>,
    },
    /// List per-host request gaps, or set one (0s removes it)
    Rate {
        #[arg(value_parser = config::validate_host)]
        host: Option<String>,
        #[arg(value_parser = humantime::parse_duration, requires = "host")]
        gap: Option<Duration>,
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
        } else if err.is::<get::RateLimited>() {
            ExitCode::from(get::RATE_LIMITED)
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
            plain,
            section,
            outline,
            max_wait,
            quiet,
        } => {
            get::cache_key(&url)?;
            let store = Store::open(&db_path())?;
            let format = match (raw, plain) {
                (true, _) => Format::Raw,
                (false, true) => Format::Plain,
                (false, false) => Format::Markdown,
            };
            let select = match (section, outline) {
                (Some(heading), _) => Select::Section(heading),
                (None, true) => Select::Outline,
                (None, false) => Select::Whole,
            };
            get::run(
                &store,
                &GetArgs {
                    url,
                    ttl,
                    refresh,
                    format,
                    select,
                    max_wait,
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
        Command::Config {
            setting: Setting::Rate { host, gap },
        } => rate_setting(host.as_deref(), gap),
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

fn rate_setting(host: Option<&str>, gap: Option<Duration>) -> Result<ExitCode> {
    if let (Some(host), Some(gap)) = (host, gap) {
        config::set_rate(host, gap)?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut listing = String::new();
    for (known, gap) in config::rate_overrides()? {
        if host.is_none_or(|wanted| wanted == known) {
            let _ = writeln!(listing, "{known} {}", humantime::format_duration(gap));
        }
    }
    noted::output::emit(listing.as_bytes())?;
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
