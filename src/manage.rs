use std::fmt::Write;
use std::path::Path;

use anyhow::Result;

use crate::cache::{Store, now};
use crate::config;
use crate::get::cache_key;
use crate::output::{emit, short_duration, short_size};

pub fn ls(store: &Store) -> Result<()> {
    let now = now();
    let mut listing = String::new();
    for entry in store.list()? {
        writeln!(
            listing,
            "{} {} {} {} {} {}",
            short_duration(now - entry.fetched_at),
            short_duration(entry.ttl_secs),
            if entry.is_fresh(now) {
                "fresh"
            } else {
                "stale"
            },
            short_size(entry.size),
            entry.hits,
            entry.url,
        )?;
    }
    emit(listing.as_bytes())
}

pub fn rm(store: &Store, urls: &[String], stale: bool, all: bool) -> Result<()> {
    let removed = if all {
        store.clear()?
    } else if stale {
        store.remove_stale(now())?
    } else {
        let keys = urls
            .iter()
            .map(|url| cache_key(url))
            .collect::<Result<Vec<_>>>()?;
        let mut removed = 0;
        for key in &keys {
            removed += usize::from(store.remove(key)?);
        }
        removed
    };
    emit(format!("removed {removed}\n").as_bytes())
}

pub fn stats(store: &Store, path: &Path) -> Result<()> {
    let now = now();
    let entries = store.list()?;
    let stale = entries.iter().filter(|e| !e.is_fresh(now)).count();
    let bytes: i64 = entries.iter().map(|e| e.size).sum();
    let hits: i64 = entries.iter().map(|e| e.hits).sum();
    let agent = match config::user_agent()? {
        Some((value, source)) => format!("{value} ({})", source.label()),
        None => "not set".to_owned(),
    };
    emit(
        format!(
            "entries: {}\nstale: {stale}\nbytes: {bytes} ({})\nhits: {hits}\ndb: {}\nuser agent: {agent}\n",
            entries.len(),
            short_size(bytes),
            path.display(),
        )
        .as_bytes(),
    )
}
