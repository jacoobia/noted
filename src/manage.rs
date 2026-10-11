use std::fmt::Write;
use std::path::Path;

use anyhow::Result;

use crate::cache::{Store, now};
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
        let mut removed = 0;
        for url in urls {
            removed += usize::from(store.remove(&cache_key(url)?)?);
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
    emit(
        format!(
            "entries: {}\nstale: {stale}\nbytes: {bytes} ({})\nhits: {hits}\ndb: {}\n",
            entries.len(),
            short_size(bytes),
            path.display(),
        )
        .as_bytes(),
    )
}
