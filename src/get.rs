use std::io::Write;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, bail};
use url::Url;

use crate::cache::{Entry, Store, now};
use crate::fetch::{Response, fetch};

const DEFAULT_TTL_SECS: i64 = 24 * 60 * 60;

pub struct GetArgs {
    pub url: String,
    pub ttl: Option<Duration>,
    pub refresh: bool,
    pub raw: bool,
    pub section: Option<String>,
    pub quiet: bool,
}

pub fn cache_key(url: &str) -> Result<String> {
    let mut parsed = Url::parse(url)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("only http and https URLs are supported: {url}");
    }
    parsed.set_fragment(None);
    Ok(parsed.into())
}

pub fn run(store: &Store, args: &GetArgs) -> Result<ExitCode> {
    let key = cache_key(&args.url)?;
    let now = now();
    let cached = store.get(&key)?;
    let ttl_secs = args
        .ttl
        .map(|ttl| i64::try_from(ttl.as_secs()).unwrap_or(i64::MAX))
        .or(cached.as_ref().map(|e| e.ttl_secs))
        .unwrap_or(DEFAULT_TTL_SECS);

    let Some(mut entry) = cached else {
        return match fetch(&key, None, None)? {
            response if is_success(response.status) => {
                let entry = store_response(store, key, response, now, ttl_secs)?;
                serve(&entry, "miss", now, args)
            }
            response => http_error(response.status, &response.body),
        };
    };

    entry.ttl_secs = ttl_secs;
    if !args.refresh && entry.is_fresh(now) {
        if args.ttl.is_some() {
            store.touch(&key, entry.fetched_at, ttl_secs)?;
        }
        store.record_hit(&key)?;
        return serve(&entry, "hit", now, args);
    }

    match fetch(&key, entry.etag.as_deref(), entry.last_modified.as_deref()) {
        Ok(response) if response.status == 304 => {
            store.touch(&key, now, ttl_secs)?;
            entry.fetched_at = now;
            serve(&entry, "revalidated", now, args)
        }
        Ok(response) if is_success(response.status) => {
            let entry = store_response(store, key, response, now, ttl_secs)?;
            serve(&entry, "refreshed", now, args)
        }
        Ok(response) if response.status >= 500 => {
            eprintln!("noted: HTTP {}, serving the stale copy", response.status);
            serve(&entry, "stale", now, args)
        }
        Err(err) => {
            eprintln!("noted: {err:#}, serving the stale copy");
            serve(&entry, "stale", now, args)
        }
        Ok(response) => http_error(response.status, &response.body),
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

fn store_response(
    store: &Store,
    url: String,
    response: Response,
    now: i64,
    ttl_secs: i64,
) -> Result<Entry> {
    let entry = Entry {
        url,
        status: response.status,
        content_type: response.content_type,
        etag: response.etag,
        last_modified: response.last_modified,
        body: response.body,
        fetched_at: now,
        ttl_secs,
        hits: 0,
    };
    store.put(&entry)?;
    Ok(entry)
}

fn http_error(status: u16, body: &[u8]) -> Result<ExitCode> {
    eprintln!("noted: HTTP {status}");
    std::io::stdout().write_all(body)?;
    Ok(ExitCode::FAILURE)
}

fn serve(entry: &Entry, state: &str, now: i64, args: &GetArgs) -> Result<ExitCode> {
    if !args.quiet {
        eprintln!(
            "noted: {state} · age {} · ttl {} · {} {}",
            short_duration(now - entry.fetched_at),
            short_duration(entry.ttl_secs),
            entry.status,
            entry.content_type.as_deref().unwrap_or("-"),
        );
    }
    std::io::stdout().write_all(&entry.body)?;
    Ok(ExitCode::SUCCESS)
}

pub fn short_duration(secs: i64) -> String {
    const UNITS: [(i64, &str); 4] = [(86_400, "d"), (3_600, "h"), (60, "m"), (1, "s")];
    UNITS.iter().find(|(size, _)| secs >= *size).map_or_else(
        || "0s".to_owned(),
        |(size, unit)| format!("{}{unit}", secs / size),
    )
}
