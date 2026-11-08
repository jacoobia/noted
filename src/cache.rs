use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use rusqlite::{
    Connection, ErrorCode, OptionalExtension, Row, Transaction, TransactionBehavior, params,
};

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
    PRAGMA secure_delete = ON;
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
    CREATE TABLE IF NOT EXISTS host_slots (
        host TEXT PRIMARY KEY,
        last_at_ms INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS robots (
        origin TEXT PRIMARY KEY,
        body TEXT NOT NULL,
        fetched_at INTEGER NOT NULL
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
        is_fresh(self.fetched_at, self.ttl_secs, now)
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
        is_fresh(self.fetched_at, self.ttl_secs, now)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Booked { wait_ms: i64 },
    TooFar { wait_ms: i64 },
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
        conn.busy_timeout(BUSY_TIMEOUT)?;
        enable_wal(&conn)?;
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
             FROM entries ORDER BY fetched_at DESC, rowid DESC",
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
        let removed = self
            .conn
            .execute("DELETE FROM entries WHERE url = ?1", [url])?;
        self.flush_wal()?;
        Ok(removed > 0)
    }

    pub fn remove_stale(&self, now: i64) -> Result<usize> {
        let removed = self.conn.execute(
            "DELETE FROM entries WHERE fetched_at + ttl_secs <= ?1",
            [now],
        )?;
        self.flush_wal()?;
        Ok(removed)
    }

    pub fn robots(&self, origin: &str) -> Result<Option<(String, i64)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT body, fetched_at FROM robots WHERE origin = ?1",
                [origin],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    pub fn put_robots(&self, origin: &str, body: &str, fetched_at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO robots (origin, body, fetched_at) VALUES (?1, ?2, ?3)",
            params![origin, body, fetched_at],
        )?;
        Ok(())
    }

    pub fn reserve_slot(&self, host: &str, gap_ms: i64, max_wait_ms: i64) -> Result<Slot> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let slot = book_slot(&tx, host, gap_ms, now_ms(), max_wait_ms)?;
        tx.commit()?;
        Ok(slot)
    }

    #[cfg(test)]
    fn reserve_slot_at(
        &self,
        host: &str,
        gap_ms: i64,
        now_ms: i64,
        max_wait_ms: i64,
    ) -> Result<Slot> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let slot = book_slot(&tx, host, gap_ms, now_ms, max_wait_ms)?;
        tx.commit()?;
        Ok(slot)
    }

    pub fn clear(&self) -> Result<usize> {
        self.conn.execute("DELETE FROM robots", [])?;
        self.conn.execute("DELETE FROM host_slots", [])?;
        let removed = self.conn.execute("DELETE FROM entries", [])?;
        self.flush_wal()?;
        Ok(removed)
    }

    fn flush_wal(&self) -> Result<()> {
        self.conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        Ok(())
    }
}

/// When several processes open a new cache at once, they race to switch it to WAL. SQLite
/// answers the losers with `SQLITE_BUSY` straight away instead of using the busy handler, so the
/// switch is retried here for as long as the busy timeout would have waited.
fn enable_wal(conn: &Connection) -> Result<()> {
    let deadline = Instant::now() + BUSY_TIMEOUT;
    loop {
        match conn.execute_batch("PRAGMA journal_mode = WAL;") {
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == ErrorCode::DatabaseBusy && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            result => return Ok(result?),
        }
    }
}

fn is_fresh(fetched_at: i64, ttl_secs: i64, now: i64) -> bool {
    now < fetched_at.saturating_add(ttl_secs)
}

pub fn db_path() -> PathBuf {
    std::env::var_os("NOTED_DB")
        .filter(|path| !path.is_empty())
        .map_or_else(
            || {
                dirs::cache_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("noted")
                    .join("cache.db")
            },
            PathBuf::from,
        )
}

fn book_slot(
    conn: &Connection,
    host: &str,
    gap_ms: i64,
    now_ms: i64,
    max_wait_ms: i64,
) -> Result<Slot> {
    let last_at: Option<i64> = conn
        .query_row(
            "SELECT last_at_ms FROM host_slots WHERE host = ?1",
            [host],
            |row| row.get(0),
        )
        .optional()?;
    let slot_at = last_at.map_or(now_ms, |last| last.saturating_add(gap_ms).max(now_ms));
    let wait_ms = slot_at - now_ms;
    if wait_ms > max_wait_ms {
        return Ok(Slot::TooFar { wait_ms });
    }
    conn.execute(
        "INSERT OR REPLACE INTO host_slots (host, last_at_ms) VALUES (?1, ?2)",
        params![host, slot_at],
    )?;
    Ok(Slot::Booked { wait_ms })
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis().try_into().unwrap_or(i64::MAX))
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
    fn huge_ttl_never_expires() {
        let mut e = entry("https://a.io/", 100);
        e.ttl_secs = i64::MAX;
        assert!(e.is_fresh(i64::MAX - 1));
    }

    #[test]
    fn robots_rules_round_trip_and_replace() {
        let (_dir, store) = store();
        assert_eq!(store.robots("https://a.io").unwrap(), None);
        store
            .put_robots("https://a.io", "User-agent: *", 100)
            .unwrap();
        store
            .put_robots("https://a.io", "Disallow: /x", 200)
            .unwrap();
        assert_eq!(
            store.robots("https://a.io").unwrap(),
            Some(("Disallow: /x".to_owned(), 200))
        );
    }

    #[test]
    fn clear_also_forgets_robots_rules() {
        let (_dir, store) = store();
        store.put(&entry("https://a.io/", 0)).unwrap();
        store.put_robots("https://a.io", "Disallow: /", 0).unwrap();
        assert_eq!(store.clear().unwrap(), 1);
        assert_eq!(store.robots("https://a.io").unwrap(), None);
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn slots_queue_up_and_refuse_long_waits_without_booking() {
        let (_dir, store) = store();
        let book = |host, gap, now, max| store.reserve_slot_at(host, gap, now, max).unwrap();
        assert_eq!(book("a.io", 1000, 0, 5000), Slot::Booked { wait_ms: 0 });
        assert_eq!(book("a.io", 1000, 0, 5000), Slot::Booked { wait_ms: 1000 });
        assert_eq!(book("a.io", 1000, 0, 500), Slot::TooFar { wait_ms: 2000 });
        assert_eq!(book("a.io", 1000, 0, 5000), Slot::Booked { wait_ms: 2000 });
        assert_eq!(book("b.io", 1000, 0, 5000), Slot::Booked { wait_ms: 0 });
    }

    #[test]
    fn a_changed_gap_applies_to_the_next_booking() {
        let (_dir, store) = store();
        store.reserve_slot_at("a.io", 60_000, 0, 5000).unwrap();
        assert_eq!(
            store.reserve_slot_at("a.io", 1000, 0, 5000).unwrap(),
            Slot::Booked { wait_ms: 1000 }
        );
    }

    #[test]
    fn clear_resets_slots() {
        let (_dir, store) = store();
        store.reserve_slot_at("a.io", 60_000, 0, 5000).unwrap();
        store.clear().unwrap();
        assert_eq!(
            store.reserve_slot_at("a.io", 60_000, 0, 5000).unwrap(),
            Slot::Booked { wait_ms: 0 }
        );
    }

    #[test]
    fn opens_in_wal_mode() {
        let (_dir, store) = store();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn remove_stale_keeps_fresh() {
        let (_dir, store) = store();
        store.put(&entry("https://old.io/", 0)).unwrap();
        store.put(&entry("https://edge.io/", 60)).unwrap();
        store.put(&entry("https://new.io/", 100)).unwrap();
        assert!(
            !store
                .get("https://edge.io/")
                .unwrap()
                .unwrap()
                .is_fresh(120)
        );
        assert_eq!(store.remove_stale(120).unwrap(), 2);
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

    #[test]
    fn list_breaks_ties_by_latest_write() {
        let (_dir, store) = store();
        store.put(&entry("https://first.io/", 100)).unwrap();
        store.put(&entry("https://second.io/", 100)).unwrap();
        assert_eq!(store.list().unwrap()[0].url, "https://second.io/");
    }
}
