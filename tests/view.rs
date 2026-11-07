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

#[test]
fn closed_stdout_exits_quietly() {
    let server = TestServer::start(vec![(
        "/big",
        always(|| Reply::ok("text/plain", "x".repeat(1 << 20))),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let mut child = common::noted_process(&db)
        .args(["get", "-q", &server.url("/big")])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn empty_section_is_rejected() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "--section", " ", "https://a.io/"])
        .assert()
        .code(64);
}

#[test]
fn section_works_on_plain_text() {
    let server = TestServer::start(vec![(
        "/notes.md",
        always(|| {
            Reply::ok(
                "text/markdown",
                "# Notes
## Keep
yes
## Skip
no
",
            )
        }),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--section", "keep", &server.url("/notes.md")])
        .assert()
        .success()
        .stdout(
            "## Keep
yes
",
        );
}

#[test]
fn section_with_raw_still_reads_markdown() {
    let dir = TempDir::new().unwrap();
    let server = wiki_server();
    noted(&dir.path().join("c.db"))
        .args([
            "get",
            "-q",
            "--raw",
            "--section",
            "trivia",
            &server.url("/w/Abyssal_demon"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("## Trivia"));
}

#[test]
fn outline_lists_the_page_headings() {
    let dir = TempDir::new().unwrap();
    let server = wiki_server();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--outline", &server.url("/w/Abyssal_demon")])
        .assert()
        .success()
        .stdout("## Drops\n### 100%\n## Trivia\n");
}

#[test]
fn outline_and_section_cannot_be_combined() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "--outline", "--section", "Drops", "https://a.io/"])
        .assert()
        .code(64);
}

#[test]
fn outline_lines_are_clean_titles_that_section_accepts() {
    let server = TestServer::start(vec![(
        "/doc",
        always(|| {
            Reply::ok(
                "text/html",
                "<body><h1>Array.prototype.at()</h1><h2><a href=\"#description\">Description</a></h2><p>x</p></body>",
            )
        }),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--outline", &server.url("/doc")])
        .assert()
        .success()
        .stdout("# Array.prototype.at()\n## Description\n");
}

#[test]
fn outline_says_when_a_page_has_no_headings() {
    let server = TestServer::start(vec![(
        "/flat",
        always(|| Reply::ok("text/plain", "just text\n")),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--outline", &server.url("/flat")])
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("no headings"));
}

#[test]
fn outline_rejects_binary_content() {
    let server = TestServer::start(vec![(
        "/data",
        always(|| Reply::ok("application/octet-stream", vec![0, 1, 2])),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "-q", "--outline", &server.url("/data")])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("--outline"));
}

#[test]
fn plain_strips_links_but_keeps_their_text_and_code() {
    let server = TestServer::start(vec![(
        "/links",
        always(|| {
            Reply::ok(
                "text/html",
                "<body><h2><a href=\"#drops\">Drops</a></h2>\
                 <p>See <a href=\"/w/Death_rune\">Death rune</a> and <code>[a](b)</code>.</p>\
                 <table><tr><th>Item</th></tr><tr><td><a href=\"/w/Ashes\">Ashes</a></td></tr></table></body>",
            )
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", "-q", "--plain", &server.url("/links")])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("See Death rune and `[a](b)`.")
                .and(predicate::str::contains("## Drops"))
                .and(predicate::str::contains("| Ashes |"))
                .and(predicate::str::contains("](/w/").not()),
        );
    noted(&db)
        .args(["get", "-q", "--plain", "--outline", &server.url("/links")])
        .assert()
        .success()
        .stdout("## Drops\n");
}

#[test]
fn plain_leaves_non_html_untouched_and_conflicts_with_raw() {
    let server = TestServer::start(vec![(
        "/api",
        always(|| Reply::ok("application/json", "{\"link\":\"[a](b)\"}")),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", "-q", "--plain", &server.url("/api")])
        .assert()
        .success()
        .stdout("{\"link\":\"[a](b)\"}");
    noted(&db)
        .args(["get", "--plain", "--raw", &server.url("/api")])
        .assert()
        .code(64);
}
