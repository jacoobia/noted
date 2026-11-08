use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use url::Url;

use crate::cache::{Entry, Slot, Store, now, now_ms};
use crate::config::{rate_for, require_user_agent};
use crate::fetch::{Response, fetch};
use crate::output::{emit, short_duration, short_size};
use crate::robots::{crawl_delay, is_allowed, path_and_query, product_token, robots_url};
use crate::view::{html_to_markdown, is_html, outline, plain, section};

const DEFAULT_TTL_SECS: i64 = 24 * 60 * 60;
const ROBOTS_TTL_SECS: i64 = 24 * 60 * 60;
const ROBOTS_REDIRECTS: u32 = 5;
const MAX_PAGE_REDIRECTS: u32 = 10;

pub struct GetArgs {
    pub url: String,
    pub ttl: Option<Duration>,
    pub refresh: bool,
    pub format: Format,
    pub select: Select,
    pub max_wait: Duration,
    pub quiet: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Plain,
    Raw,
}

pub enum Select {
    Whole,
    Section(String),
    Outline,
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
        let session = Session::new(store, args, now)?;
        return match session.fetch_page(&key, None, None)? {
            Fetched::Blocked(Blocked::Disallowed(reason)) => bail!(reason),
            Fetched::Blocked(Blocked::Unreachable(why)) => {
                bail!("couldn't check robots.txt ({why}), so nothing was fetched")
            }
            Fetched::Page(response) if is_success(response.status) => {
                let entry = store_response(store, key, response, now, ttl_secs)?;
                serve(&entry, "miss", now, args)
            }
            Fetched::Page(response) => Ok(http_error(response.status)),
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

    let session = Session::new(store, args, now)?;
    match session.fetch_page(&key, entry.etag.as_deref(), entry.last_modified.as_deref()) {
        Ok(Fetched::Blocked(Blocked::Disallowed(reason))) => bail!(reason),
        Ok(Fetched::Blocked(Blocked::Unreachable(why))) => {
            eprintln!("noted: couldn't check robots.txt ({why}), serving the stale copy");
            serve(&entry, "stale", now, args)
        }
        Ok(Fetched::Page(response)) if response.status == 304 => {
            store.touch(&key, now, ttl_secs)?;
            entry.fetched_at = now;
            serve(&entry, "revalidated", now, args)
        }
        Ok(Fetched::Page(response)) if is_success(response.status) => {
            let entry = store_response(store, key, response, now, ttl_secs)?;
            serve(&entry, "refreshed", now, args)
        }
        Ok(Fetched::Page(response)) if response.status >= 500 => {
            eprintln!("noted: HTTP {}, serving the stale copy", response.status);
            serve(&entry, "stale", now, args)
        }
        Err(err) if is_too_large(&err) || err.is::<RateLimited>() => Err(err),
        Err(err) => {
            eprintln!("noted: {err:#}, serving the stale copy");
            serve(&entry, "stale", now, args)
        }
        Ok(Fetched::Page(response)) => Ok(http_error(response.status)),
    }
}

pub const RATE_LIMITED: u8 = 4;

#[derive(Debug)]
pub struct RateLimited {
    host: String,
    retry_after: Duration,
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} is rate limited, retry after {} (or raise --max-wait)",
            self.host,
            humantime::format_duration(self.retry_after)
        )
    }
}

impl std::error::Error for RateLimited {}

enum Fetched {
    Page(Response),
    Blocked(Blocked),
}

enum Blocked {
    Disallowed(String),
    Unreachable(String),
}

struct Session<'a> {
    store: &'a Store,
    agent: String,
    now: i64,
    max_wait: Duration,
    quiet: bool,
}

impl<'a> Session<'a> {
    fn new(store: &'a Store, args: &GetArgs, now: i64) -> Result<Self> {
        Ok(Self {
            store,
            agent: require_user_agent()?,
            now,
            max_wait: args.max_wait,
            quiet: args.quiet,
        })
    }

    fn fetch_page(
        &self,
        start: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<Fetched> {
        let mut url = start.to_owned();
        for _ in 0..=MAX_PAGE_REDIRECTS {
            if let Some(blocked) = self.check_robots(&url)? {
                return Ok(Fetched::Blocked(blocked));
            }
            let response = self.request(&url, etag, last_modified, 0)?;
            match redirect_target(&url, &response) {
                Some(next) => url = next,
                None => return Ok(Fetched::Page(response)),
            }
        }
        bail!("stopped after {MAX_PAGE_REDIRECTS} redirects from {start}")
    }

    fn check_robots(&self, page: &str) -> Result<Option<Blocked>> {
        let page = Url::parse(page)?;
        if page.path() == "/robots.txt" {
            return Ok(None);
        }
        let url = robots_url(&page);
        let origin = page.origin().ascii_serialization();
        let cached = self.store.robots(&origin)?;
        let rules = match cached {
            Some((body, fetched_at)) if self.now < fetched_at.saturating_add(ROBOTS_TTL_SECS) => {
                body
            }
            stale => match fetch(&url, &self.agent, None, None, ROBOTS_REDIRECTS) {
                Ok(response) if response.status < 500 => {
                    let body = if is_success(response.status) {
                        String::from_utf8_lossy(&response.body).into_owned()
                    } else {
                        String::new()
                    };
                    self.store.put_robots(&origin, &body, self.now)?;
                    body
                }
                Ok(response) => match stale {
                    Some((body, _)) => body,
                    None => {
                        return Ok(Some(Blocked::Unreachable(format!(
                            "HTTP {}",
                            response.status
                        ))));
                    }
                },
                Err(err) if err.is::<RateLimited>() => return Err(err),
                Err(err) => match stale {
                    Some((body, _)) => body,
                    None => return Ok(Some(Blocked::Unreachable(format!("{err:#}")))),
                },
            },
        };
        let token = product_token(&self.agent);
        let path = path_and_query(&page);
        if is_allowed(&rules, token, &path) {
            Ok(None)
        } else {
            let host = page.host_str().unwrap_or_default();
            Ok(Some(Blocked::Disallowed(format!(
                "robots.txt on {host} disallows {path} for {token}, so nothing was fetched"
            ))))
        }
    }

    fn request(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        max_redirects: u32,
    ) -> Result<Response> {
        self.wait_for_slot(url)?;
        fetch(url, &self.agent, etag, last_modified, max_redirects)
    }

    fn wait_for_slot(&self, url: &str) -> Result<()> {
        let page = Url::parse(url)?;
        let Some(host) = page.host_str() else {
            return Ok(());
        };
        let Some(gap) = self.gap_for(&page, host)? else {
            return Ok(());
        };
        let slot = self
            .store
            .reserve_slot(host, millis(gap), now_ms(), millis(self.max_wait))?;
        match slot {
            Slot::Booked { wait_ms } if wait_ms > 0 => {
                let wait = Duration::from_millis(wait_ms.unsigned_abs());
                if !self.quiet {
                    eprintln!(
                        "noted: waiting {} for {host} (gap {})",
                        humantime::format_duration(round_up_to_seconds(wait)),
                        humantime::format_duration(gap)
                    );
                }
                thread::sleep(wait);
                Ok(())
            }
            Slot::Booked { .. } => Ok(()),
            Slot::TooFar { wait_ms } => bail!(RateLimited {
                host: host.to_owned(),
                retry_after: round_up_to_seconds(Duration::from_millis(wait_ms.unsigned_abs())),
            }),
        }
    }

    fn gap_for(&self, page: &Url, host: &str) -> Result<Option<Duration>> {
        if let Some(gap) = rate_for(host)? {
            return Ok(Some(gap));
        }
        let origin = page.origin().ascii_serialization();
        Ok(self
            .store
            .robots(&origin)?
            .and_then(|(rules, _)| crawl_delay(&rules, product_token(&self.agent)))
            .filter(|gap| !gap.is_zero()))
    }
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

fn round_up_to_seconds(duration: Duration) -> Duration {
    Duration::from_secs(
        duration
            .as_millis()
            .div_ceil(1000)
            .try_into()
            .unwrap_or(u64::MAX),
    )
}

fn redirect_target(current: &str, response: &Response) -> Option<String> {
    if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let next = Url::parse(current)
        .ok()?
        .join(response.location.as_deref()?)
        .ok()?;
    matches!(next.scheme(), "http" | "https").then(|| next.into())
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

fn http_error(status: u16) -> ExitCode {
    eprintln!("noted: HTTP {status}, nothing printed");
    ExitCode::FAILURE
}

fn serve(entry: &Entry, state: &str, now: i64, args: &GetArgs) -> Result<ExitCode> {
    if !args.quiet {
        eprintln!(
            "noted: {state} · age {} · ttl {} · {} {} · {}",
            short_duration(now - entry.fetched_at),
            short_duration(entry.ttl_secs),
            entry.status,
            entry.content_type.as_deref().unwrap_or("-"),
            short_size(i64::try_from(entry.body.len()).unwrap_or(i64::MAX)),
        );
    }
    let content_type = entry.content_type.as_deref();
    let html = is_html(content_type);
    if !matches!(args.select, Select::Whole) {
        if !html
            && !content_type.is_some_and(|ct| {
                ct.get(..5)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("text/"))
            })
        {
            bail!(
                "--section and --outline need an HTML or text response, got {}",
                content_type.unwrap_or("no content type")
            );
        }
        let text = String::from_utf8_lossy(&entry.body);
        let text = if html {
            html_to_markdown(&text)
        } else {
            text.into_owned()
        };
        let text = if html && args.format == Format::Plain {
            plain(&text)
        } else {
            text
        };
        let Select::Section(heading) = &args.select else {
            let headings = outline(&text);
            if headings.is_empty() {
                eprintln!("noted: no headings on this page");
            } else {
                emit(format!("{}\n", headings.join("\n")).as_bytes())?;
            }
            return Ok(ExitCode::SUCCESS);
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
    if html && args.format != Format::Raw {
        let markdown = html_to_markdown(&String::from_utf8_lossy(&entry.body));
        let markdown = if args.format == Format::Plain {
            plain(&markdown)
        } else {
            markdown
        };
        emit(format!("{markdown}\n").as_bytes())?;
    } else {
        emit(&entry.body)?;
    }
    Ok(ExitCode::SUCCESS)
}
