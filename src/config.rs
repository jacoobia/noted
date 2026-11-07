use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

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
    if let Ok(value) = std::env::var("NOTED_USER_AGENT") {
        let value =
            validate_user_agent(&value).map_err(|err| anyhow::anyhow!("NOTED_USER_AGENT {err}"))?;
        return Ok(Some((value, Source::Env)));
    }
    let path = user_agent_path();
    match fs::read_to_string(&path) {
        Ok(saved) => {
            let value = validate_user_agent(&saved)
                .map_err(|err| anyhow::anyhow!("{} {err}", path.display()))?;
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
    Ok(value.to_owned())
}

pub fn require_user_agent() -> Result<String> {
    match user_agent()? {
        Some((value, _)) => Ok(value),
        None => bail!(SetupNeeded),
    }
}

#[derive(Debug)]
pub struct SetupNeeded;

impl std::fmt::Display for SetupNeeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no User-Agent is set, so nothing was fetched.\n\
             Ask the user how requests should identify themselves, offering three choices:\n  \
             1. Recommended: <Agent>/<version> (+mailto:<their contact>), e.g. Claude-Code/2.1 (+mailto:you@example.com)\n  \
             2. Their own string\n  \
             3. The User-Agent this agent normally sends\n\
             Then save it with: noted config user-agent \"<value>\""
        )
    }
}

impl std::error::Error for SetupNeeded {}

fn user_agent_path() -> PathBuf {
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
        .join("user-agent")
}
