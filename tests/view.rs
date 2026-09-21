mod common;

use common::{Reply, TestServer, always, noted};
use predicates::prelude::*;
use tempfile::TempDir;

const WIKI: &str = include_str!("fixtures/wiki.html");

fn wiki_server() -> TestServer {
    TestServer::start(vec![(
        "/w/Abyssal_demon",
        always(|| Reply::ok("text/html; charset=utf-8", WIKI)),
    )])
}

#[test]
fn html_prints_as_markdown() {
    let dir = TempDir::new().unwrap();
    let server = wiki_server();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", &server.url("/w/Abyssal_demon")])
        .assert()
        .success()
        .stdout(predicate::str::contains("## Drops").and(predicate::str::contains("<div").not()));
}

#[test]
fn raw_prints_original_html() {
    let dir = TempDir::new().unwrap();
    let server = wiki_server();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--raw", &server.url("/w/Abyssal_demon")])
        .assert()
        .success()
        .stdout(WIKI);
}
