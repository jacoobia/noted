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

#[test]
fn section_prints_one_part_of_the_page() {
    let dir = TempDir::new().unwrap();
    let server = wiki_server();
    noted(&dir.path().join("c.db"))
        .args([
            "get",
            "-q",
            "--section",
            "drops",
            &server.url("/w/Abyssal_demon"),
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::starts_with("## Drops")
                .and(predicate::str::contains("| Ashes | 1 |"))
                .and(predicate::str::contains("Trivia").not()),
        );
}

#[test]
fn missing_section_lists_headings() {
    let dir = TempDir::new().unwrap();
    let server = wiki_server();
    noted(&dir.path().join("c.db"))
        .args([
            "get",
            "-q",
            "--section",
            "Nope",
            &server.url("/w/Abyssal_demon"),
        ])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("Drops, 100%, Trivia"));
}

#[test]
fn section_rejects_binary_content() {
    let server = TestServer::start(vec![(
        "/data",
        always(|| Reply::ok("application/octet-stream", vec![0, 1, 2])),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--section", "x", &server.url("/data")])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--section"));
}
