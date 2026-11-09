use std::fmt::Write;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::robots::product_token;

pub const SETUP_NEEDED: u8 = 3;
const MAX_USER_AGENT: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Config,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Env => "NOTED_USER_AGENT",
            Source::Config => "config file",
        }
    }
}

pub fn user_agent() -> Result<Option<(String, Source)>> {
    match std::env::var("NOTED_USER_AGENT") {
        Ok(value) => {
            let value = validate_user_agent(&value).map_err(InvalidEnvUserAgent)?;
            return Ok(Some((value, Source::Env)));
        }
        Err(std::env::VarError::NotUnicode(_)) => {
            bail!(InvalidEnvUserAgent("must be printable ASCII".to_owned()));
        }
        Err(std::env::VarError::NotPresent) => {}
    }
    let path = user_agent_path();
    match fs::read_to_string(&path) {
        Ok(saved) => {
            let value = validate_user_agent(&saved).map_err(|reason| SetupNeeded {
                invalid: Some(format!("{} {reason}", path.display())),
            })?;
            Ok(Some((value, Source::Config)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn save_user_agent(value: &str) -> Result<PathBuf> {
    let path = user_agent_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, format!("{value}\n"))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn validate_user_agent(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("must not be blank".to_owned());
    }
    if value.len() > MAX_USER_AGENT {
        return Err(format!("must be at most {MAX_USER_AGENT} characters"));
    }
    if !value.bytes().all(|b| (b' '..=b'~').contains(&b)) {
        return Err("must be printable ASCII with no control characters".to_owned());
    }
    if product_token(value).is_empty() {
        return Err("must start with a product name, like Claude-Code/2.1".to_owned());
    }
    Ok(value.to_owned())
}

pub fn require_user_agent() -> Result<String> {
    match user_agent()? {
        Some((value, _)) => Ok(value),
        None => bail!(SetupNeeded { invalid: None }),
    }
}

#[derive(Debug)]
pub struct InvalidEnvUserAgent(String);

impl std::fmt::Display for InvalidEnvUserAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NOTED_USER_AGENT {}", self.0)
    }
}

impl std::error::Error for InvalidEnvUserAgent {}

#[derive(Debug)]
pub struct SetupNeeded {
    invalid: Option<String>,
}

impl std::fmt::Display for SetupNeeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.invalid {
            Some(reason) => writeln!(f, "the saved User-Agent is invalid: {reason}.")?,
            None => writeln!(f, "no User-Agent is set, so nothing was fetched.")?,
        }
        write!(
            f,
            "Ask the user how requests should identify themselves, offering three choices:\n  \
             1. Recommended: <Agent>/<version> (+mailto:<their contact>), e.g. Claude-Code/2.1 (+mailto:you@example.com)\n  \
             2. Their own string\n  \
             3. The User-Agent this agent normally sends\n\
             Then save it with: noted config user-agent \"<value>\""
        )
    }
}

impl std::error::Error for SetupNeeded {}

struct RateFile {
    overrides: Vec<(String, Duration)>,
    unreadable: Vec<String>,
}

fn read_rate_file() -> Result<RateFile> {
    let path = rate_limits_path();
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err).with_context(|| format!("reading {}", path.display())),
    };
    let mut file = RateFile {
        overrides: Vec::new(),
        unreadable: Vec::new(),
    };
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let parsed = line
            .trim()
            .split_once(char::is_whitespace)
            .and_then(|(host, gap)| {
                let host = validate_host(host).ok()?;
                let gap = humantime::parse_duration(gap.trim()).ok()?;
                Some((host, gap))
            });
        if let Some(entry) = parsed {
            file.overrides.push(entry);
        } else {
            eprintln!(
                "noted: ignoring unreadable line in {}: {line}",
                path.display()
            );
            file.unreadable.push(line.to_owned());
        }
    }
    file.overrides.sort();
    Ok(file)
}

pub fn rate_overrides() -> Result<Vec<(String, Duration)>> {
    Ok(read_rate_file()?.overrides)
}

pub fn rate_for(host: &str) -> Result<Option<Duration>> {
    let host = host.to_ascii_lowercase();
    Ok(rate_overrides()?
        .into_iter()
        .find(|(known, _)| *known == host)
        .map(|(_, gap)| gap))
}

pub fn set_rate(host: &str, gap: Duration) -> Result<()> {
    let file = read_rate_file()?;
    let mut overrides: Vec<_> = file
        .overrides
        .into_iter()
        .filter(|(known, _)| known != host)
        .collect();
    if !gap.is_zero() {
        overrides.push((host.to_owned(), gap));
        overrides.sort();
    }
    let mut text = String::new();
    for (host, gap) in &overrides {
        let _ = writeln!(text, "{host} {}", humantime::format_duration(*gap));
    }
    for line in &file.unreadable {
        let _ = writeln!(text, "{line}");
    }
    let path = rate_limits_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let partial = path.with_extension("partial");
    fs::write(&partial, text).with_context(|| format!("writing {}", partial.display()))?;
    fs::rename(&partial, &path).with_context(|| format!("replacing {}", path.display()))
}

pub fn validate_host(value: &str) -> Result<String, String> {
    let value = value.trim();
    let host = (!value.is_empty() && !value.contains([' ', '/']))
        .then(|| url::Host::parse(&value.to_ascii_lowercase()).ok())
        .flatten();
    host.map(|host| host.to_string())
        .ok_or_else(|| "must be a host name like en.wikipedia.org".to_owned())
}

fn rate_limits_path() -> PathBuf {
    config_dir().join("rate-limits")
}

fn user_agent_path() -> PathBuf {
    config_dir().join("user-agent")
}

fn config_dir() -> PathBuf {
    std::env::var_os("NOTED_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(
            || {
                dirs::config_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("noted")
            },
            PathBuf::from,
        )
}
