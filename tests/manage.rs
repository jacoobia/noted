mod common;

use std::path::Path;

use common::{Reply, TestServer, always, noted};
use predicates::prelude::*;
use tempfile::TempDir;

fn server() -> TestServer {
    TestServer::start(vec![
        ("/a", always(|| Reply::ok("text/plain", "aaaa"))),
        ("/b", always(|| Reply::ok("text/plain", "bb"))),
    ])
}

fn fetch(db: &Path, url: &str, ttl: &str) {
    noted(db)
        .args(["get", "-q", "--ttl", ttl, url])
        .assert()
        .success();
}

fn stdout_of(db: &Path, args: &[&str]) -> String {
    let out = noted(db).args(args).assert().success().get_output().clone();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn ls_shows_entries_newest_first() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "1d");
    fetch(&db, &server.url("/b"), "1d");
    let listing = stdout_of(&db, &["ls"]);
    let lines: Vec<_> = listing.lines().collect();
    assert_eq!(lines.len(), 2, "{listing}");
    let fields: Vec<_> = lines[0].split(' ').skip(1).collect();
    assert_eq!(
        fields,
        ["1d", "fresh", "2B", "0", &server.url("/b")],
        "{listing}"
    );
    assert!(lines[1].ends_with(&server.url("/a")), "{listing}");
}

#[test]
fn ls_marks_stale() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "0s");
    assert!(stdout_of(&db, &["ls"]).contains("stale"));
}

#[test]
fn rm_by_url_normalises_fragment() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "1d");
    let url = format!("{}#Anatomy", server.url("/a"));
    assert_eq!(stdout_of(&db, &["rm", &url]), "removed 1\n");
    assert_eq!(stdout_of(&db, &["ls"]), "");
}

#[test]
fn rm_stale_only() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "0s");
    fetch(&db, &server.url("/b"), "1d");
    assert_eq!(stdout_of(&db, &["rm", "--stale"]), "removed 1\n");
    assert!(stdout_of(&db, &["ls"]).contains("/b"));
}

#[test]
fn rm_all() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "1d");
    fetch(&db, &server.url("/b"), "1d");
    assert_eq!(stdout_of(&db, &["rm", "--all"]), "removed 2\n");
}

#[test]
fn rm_requires_one_kind_of_target() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db).arg("rm").assert().code(64);
    noted(&db)
        .args(["rm", "--all", "--stale"])
        .assert()
        .code(64);
    noted(&db)
        .args(["rm", "https://a.io/", "--all"])
        .assert()
        .code(64);
}

#[test]
fn stats_counts() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "0s");
    fetch(&db, &server.url("/b"), "1d");
    fetch(&db, &server.url("/b"), "1d");
    noted(&db).arg("stats").assert().success().stdout(
        predicate::str::contains("entries: 2")
            .and(predicate::str::contains("stale: 1"))
            .and(predicate::str::contains("bytes: 6"))
            .and(predicate::str::contains("hits: 1"))
            .and(predicate::str::contains(db.to_string_lossy().as_ref())),
    );
}

#[test]
fn rm_invalid_url_removes_nothing() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = server();
    fetch(&db, &server.url("/a"), "1d");
    noted(&db)
        .args(["rm", &server.url("/a"), "file:///x"])
        .assert()
        .failure();
    assert!(stdout_of(&db, &["ls"]).contains("/a"));
}

#[test]
fn rm_all_leaves_no_page_bytes_on_disk() {
    let marker = "SECRET-MARKER-5f3a9c";
    let server = TestServer::start(vec![(
        "/secret",
        always(move || Reply::ok("text/plain", marker.repeat(200))),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    fetch(&db, &server.url("/secret"), "1d");
    stdout_of(&db, &["rm", "--all"]);
    for file in ["c.db", "c.db-wal"] {
        let path = dir.path().join(file);
        if let Ok(bytes) = std::fs::read(&path) {
            let text = String::from_utf8_lossy(&bytes);
            assert!(!text.contains(marker), "{file} still holds the page");
        }
    }
}

#[test]
fn stats_shows_the_user_agent_and_its_source() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "user agent: {} (NOTED_USER_AGENT)",
            common::TEST_AGENT
        )));
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("user agent: not set"));
}
