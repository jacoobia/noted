use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Result, bail};
use url::Url;

use crate::cache::{Entry, Store, now};
use crate::config::require_user_agent;
use crate::fetch::{Response, fetch};
use crate::output::{emit, short_duration};
use crate::robots::{is_allowed, path_and_query, product_token, robots_url};
use crate::view::{html_to_markdown, is_html, section};

const DEFAULT_TTL_SECS: i64 = 24 * 60 * 60;
const ROBOTS_TTL_SECS: i64 = 24 * 60 * 60;

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
        let agent = require_user_agent()?;
        match check_robots(store, &key, &agent, now)? {
            Robots::Allowed => {}
            Robots::Disallowed(reason) => bail!(reason),
            Robots::Unreachable(why) => {
                bail!("couldn't check robots.txt ({why}), so nothing was fetched")
            }
        }
        return match fetch(&key, &agent, None, None)? {
            response if is_success(response.status) => {
                let entry = store_response(store, key, response, now, ttl_secs)?;
                serve(&entry, "miss", now, args)
            }
            response => http_error(response.status, &response.body),
        };
    };

    entry.ttl_secs = ttl_secs;
    if args.ttl.is_some() {
        store.touch(&key, entry.fetched_at, ttl_secs)?;
    }
    if !args.refresh && entry.is_fresh(now) {
        store.record_hit(&key)?;
        return serve(&entry, "hit", now, args);
    }

    let agent = require_user_agent()?;
    match check_robots(store, &key, &agent, now)? {
        Robots::Allowed => {}
        Robots::Disallowed(reason) => bail!(reason),
        Robots::Unreachable(why) => {
            eprintln!("noted: couldn't check robots.txt ({why}), serving the stale copy");
            return serve(&entry, "stale", now, args);
        }
    }
    match fetch(
        &key,
        &agent,
        entry.etag.as_deref(),
        entry.last_modified.as_deref(),
    ) {
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
        Err(err) if is_too_large(&err) => Err(err),
        Err(err) => {
            eprintln!("noted: {err:#}, serving the stale copy");
            serve(&entry, "stale", now, args)
        }
        Ok(response) => http_error(response.status, &response.body),
    }
}

enum Robots {
    Allowed,
    Disallowed(String),
    Unreachable(String),
}

fn check_robots(store: &Store, page: &str, agent: &str, now: i64) -> Result<Robots> {
    let page = Url::parse(page)?;
    if page.path() == "/robots.txt" {
        return Ok(Robots::Allowed);
    }
    let url = robots_url(&page);
    let origin = page.origin().ascii_serialization();
    let cached = store.robots(&origin)?;
    let rules = match cached {
        Some((body, fetched_at)) if now < fetched_at.saturating_add(ROBOTS_TTL_SECS) => body,
        stale => match fetch(&url, agent, None, None) {
            Ok(response) if response.status < 500 => {
                let body = if is_success(response.status) {
                    String::from_utf8_lossy(&response.body).into_owned()
                } else {
                    String::new()
                };
                store.put_robots(&origin, &body, now)?;
                body
            }
            Ok(response) => match stale {
                Some((body, _)) => body,
                None => return Ok(Robots::Unreachable(format!("HTTP {}", response.status))),
            },
            Err(err) => match stale {
                Some((body, _)) => body,
                None => return Ok(Robots::Unreachable(format!("{err:#}"))),
            },
        },
    };
    let token = product_token(agent);
    let path = path_and_query(&page);
    if is_allowed(&rules, token, &path) {
        Ok(Robots::Allowed)
    } else {
        let host = page.host_str().unwrap_or_default();
        Ok(Robots::Disallowed(format!(
            "robots.txt on {host} disallows {path} for {token}, so nothing was fetched"
        )))
    }
}

fn is_success(status: u16) -> bool {
    (200..300).contains(&status)
}

fn is_too_large(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<ureq::Error>(),
        Some(ureq::Error::BodyExceedsLimit(_))
    )
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
    emit(body)?;
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
    let content_type = entry.content_type.as_deref();
    let html = is_html(content_type);
    if let Some(heading) = &args.section {
        if !html
            && !content_type.is_some_and(|ct| {
                ct.get(..5)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("text/"))
            })
        {
            bail!(
                "--section needs an HTML or text response, got {}",
                content_type.unwrap_or("no content type")
            );
        }
        let text = String::from_utf8_lossy(&entry.body);
        let text = if html {
            html_to_markdown(&text)
        } else {
            text.into_owned()
        };
        return match section(&text, heading) {
            Ok(part) => {
                emit(format!("{part}\n").as_bytes())?;
                Ok(ExitCode::SUCCESS)
            }
            Err(headings) => {
                eprintln!(
                    "noted: no section \"{}\". Headings: {}",
                    heading.trim(),
                    headings.join(", ")
                );
                Ok(ExitCode::FAILURE)
            }
        };
    }
    if html && !args.raw {
        let markdown = html_to_markdown(&String::from_utf8_lossy(&entry.body));
        emit(format!("{markdown}\n").as_bytes())?;
    } else {
        emit(&entry.body)?;
    }
    Ok(ExitCode::SUCCESS)
}
