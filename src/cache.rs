use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};

const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    CREATE TABLE IF NOT EXISTS entries (
        url TEXT PRIMARY KEY,
        status INTEGER NOT NULL,
        content_type TEXT,
        etag TEXT,
        last_modified TEXT,
        body BLOB NOT NULL,
        fetched_at INTEGER NOT NULL,
        ttl_secs INTEGER NOT NULL,
        hits INTEGER NOT NULL DEFAULT 0
    );
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub url: String,
    pub status: u16,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub body: Vec<u8>,
    pub fetched_at: i64,
    pub ttl_secs: i64,
    pub hits: i64,
}

impl Entry {
    pub fn is_fresh(&self, now: i64) -> bool {
        now < self.fetched_at + self.ttl_secs
    }

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            url: row.get("url")?,
            status: row.get("status")?,
            content_type: row.get("content_type")?,
            etag: row.get("etag")?,
            last_modified: row.get("last_modified")?,
            body: row.get("body")?,
            fetched_at: row.get("fetched_at")?,
            ttl_secs: row.get("ttl_secs")?,
            hits: row.get("hits")?,
        })
    }
}

#[derive(Debug)]
pub struct Listing {
    pub url: String,
    pub fetched_at: i64,
    pub ttl_secs: i64,
    pub size: i64,
    pub hits: i64,
}

impl Listing {
    pub fn is_fresh(&self, now: i64) -> bool {
        now < self.fetched_at + self.ttl_secs
    }
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn get(&self, url: &str) -> Result<Option<Entry>> {
        Ok(self
            .conn
            .query_row(
                "SELECT * FROM entries WHERE url = ?1",
                [url],
                Entry::from_row,
            )
            .optional()?)
    }

    pub fn put(&self, entry: &Entry) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO entries
             (url, status, content_type, etag, last_modified, body, fetched_at, ttl_secs, hits)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                entry.url,
                entry.status,
                entry.content_type,
                entry.etag,
                entry.last_modified,
                entry.body,
                entry.fetched_at,
                entry.ttl_secs,
                entry.hits,
            ],
        )?;
        Ok(())
    }

    pub fn touch(&self, url: &str, fetched_at: i64, ttl_secs: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE entries SET fetched_at = ?2, ttl_secs = ?3 WHERE url = ?1",
            params![url, fetched_at, ttl_secs],
        )?;
        Ok(())
    }

    pub fn record_hit(&self, url: &str) -> Result<()> {
        self.conn
            .execute("UPDATE entries SET hits = hits + 1 WHERE url = ?1", [url])?;
        Ok(())
    }

    pub fn list(&self) -> Result<Vec<Listing>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, fetched_at, ttl_secs, length(body), hits
             FROM entries ORDER BY fetched_at DESC, url",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Listing {
                url: row.get(0)?,
                fetched_at: row.get(1)?,
                ttl_secs: row.get(2)?,
                size: row.get(3)?,
                hits: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn remove(&self, url: &str) -> Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM entries WHERE url = ?1", [url])?
            > 0)
    }

    pub fn remove_stale(&self, now: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM entries WHERE fetched_at + ttl_secs <= ?1",
            [now],
        )?)
    }

    pub fn clear(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM entries", [])?)
    }
}

pub fn db_path() -> PathBuf {
    std::env::var_os("NOTED_DB").map_or_else(
        || {
            dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("noted")
                .join("cache.db")
        },
        PathBuf::from,
    )
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().try_into().unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, Store) {
        let dir = TempDir::new().unwrap();
        let store = Store::open(&dir.path().join("nested/cache.db")).unwrap();
        (dir, store)
    }

    fn entry(url: &str, fetched_at: i64) -> Entry {
        Entry {
            url: url.into(),
            status: 200,
            content_type: Some("text/html".into()),
            etag: Some("\"v1\"".into()),
            last_modified: Some("Wed, 01 Jan 2026 00:00:00 GMT".into()),
            body: vec![0, 159, 255],
            fetched_at,
            ttl_secs: 60,
            hits: 0,
        }
    }

    #[test]
    fn put_then_get_round_trips() {
        let (_dir, store) = store();
        let original = entry("https://a.io/", 100);
        store.put(&original).unwrap();
        assert_eq!(store.get("https://a.io/").unwrap(), Some(original));
    }

    #[test]
    fn get_missing_is_none() {
        let (_dir, store) = store();
        assert_eq!(store.get("https://a.io/").unwrap(), None);
    }

    #[test]
    fn put_replaces_existing() {
        let (_dir, store) = store();
        store.put(&entry("https://a.io/", 100)).unwrap();
        let mut newer = entry("https://a.io/", 200);
        newer.body = b"new".to_vec();
        store.put(&newer).unwrap();
        assert_eq!(store.get("https://a.io/").unwrap(), Some(newer));
    }

    #[test]
    fn touch_updates_fetched_at_and_ttl_only() {
        let (_dir, store) = store();
        let original = entry("https://a.io/", 100);
        store.put(&original).unwrap();
        store.touch("https://a.io/", 500, 3600).unwrap();
        let touched = store.get("https://a.io/").unwrap().unwrap();
        assert_eq!(touched.fetched_at, 500);
        assert_eq!(touched.ttl_secs, 3600);
        assert_eq!(touched.body, original.body);
        assert_eq!(touched.etag, original.etag);
    }

    #[test]
    fn record_hit_increments() {
        let (_dir, store) = store();
        store.put(&entry("https://a.io/", 100)).unwrap();
        store.record_hit("https://a.io/").unwrap();
        store.record_hit("https://a.io/").unwrap();
        assert_eq!(store.get("https://a.io/").unwrap().unwrap().hits, 2);
    }

    #[test]
    fn freshness_boundary() {
        let mut e = entry("https://a.io/", 100);
        e.ttl_secs = 10;
        assert!(e.is_fresh(109));
        assert!(!e.is_fresh(110));
    }

    #[test]
    fn remove_stale_keeps_fresh() {
        let (_dir, store) = store();
        store.put(&entry("https://old.io/", 0)).unwrap();
        store.put(&entry("https://new.io/", 100)).unwrap();
        assert_eq!(store.remove_stale(120).unwrap(), 1);
        assert!(store.get("https://old.io/").unwrap().is_none());
        assert!(store.get("https://new.io/").unwrap().is_some());
    }

    #[test]
    fn remove_and_clear_report_counts() {
        let (_dir, store) = store();
        store.put(&entry("https://a.io/", 0)).unwrap();
        store.put(&entry("https://b.io/", 0)).unwrap();
        assert!(store.remove("https://a.io/").unwrap());
        assert!(!store.remove("https://a.io/").unwrap());
        assert_eq!(store.clear().unwrap(), 1);
    }

    #[test]
    fn list_newest_first_with_size() {
        let (_dir, store) = store();
        store.put(&entry("https://old.io/", 100)).unwrap();
        store.put(&entry("https://new.io/", 200)).unwrap();
        let listed = store.list().unwrap();
        let urls: Vec<_> = listed.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(urls, ["https://new.io/", "https://old.io/"]);
        assert_eq!(listed[0].size, 3);
    }
}
