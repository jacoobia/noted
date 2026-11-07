mod common;

use common::{Reply, TEST_AGENT, TestServer, always, noted};
use predicates::prelude::*;
use tempfile::TempDir;

fn site(robots: &'static str) -> TestServer {
    TestServer::start(vec![
        (
            "/robots.txt",
            always(move || Reply::ok("text/plain", robots)),
        ),
        ("/public", always(|| Reply::ok("text/plain", "public page"))),
        (
            "/private/page",
            always(|| Reply::ok("text/plain", "private page")),
        ),
    ])
}

#[test]
fn disallowed_path_is_refused_without_fetching() {
    let server = site("User-agent: *\nDisallow: /private\n");
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/private/page")])
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("robots.txt").and(predicate::str::contains("TestAgent")));
    assert_eq!(server.hits("/private/page"), 0);
}

#[test]
fn allowed_path_is_fetched() {
    let server = site("User-agent: *\nDisallow: /private\n");
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/public")])
        .assert()
        .success()
        .stdout("public page");
}

#[test]
fn named_group_overrides_star() {
    let server = site("User-agent: *\nDisallow: /\n\nUser-agent: TestAgent\nAllow: /\n");
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/private/page")])
        .assert()
        .success();
}

#[test]
fn missing_robots_txt_allows_everything() {
    let server = TestServer::start(vec![("/page", always(|| Reply::ok("text/plain", "ok")))]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/page")])
        .assert()
        .success()
        .stdout("ok");
}

#[test]
fn robots_txt_is_fetched_once_per_host() {
    let server = site("User-agent: *\nDisallow: /private\n");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", &server.url("/public")])
        .assert()
        .success();
    noted(&db)
        .args(["get", &server.url("/private/page")])
        .assert()
        .code(1);
    assert_eq!(server.hits("/robots.txt"), 1);
    assert_eq!(
        server.last_header("/robots.txt", "User-Agent").as_deref(),
        Some(TEST_AGENT)
    );
}

#[test]
fn unreachable_robots_txt_with_nothing_cached_refuses() {
    let server = TestServer::start(vec![
        ("/robots.txt", always(|| Reply::status(503))),
        ("/page", always(|| Reply::ok("text/plain", "ok"))),
    ]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/page")])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("robots.txt"));
    assert_eq!(server.hits("/page"), 0);
}

#[test]
fn fetching_robots_txt_itself_skips_the_check() {
    let server = site("User-agent: *\nDisallow: /\n");
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/robots.txt")])
        .assert()
        .success()
        .stdout("User-agent: *\nDisallow: /\n");
    assert_eq!(server.hits("/robots.txt"), 1);
}
