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

#[test]
fn redirect_into_a_disallowed_path_is_refused() {
    let server = TestServer::start(vec![
        (
            "/robots.txt",
            always(|| Reply::ok("text/plain", "User-agent: *\nDisallow: /private\n")),
        ),
        (
            "/go",
            always(|| Reply::status(302).header("Location", "/private/page")),
        ),
        (
            "/private/page",
            always(|| Reply::ok("text/plain", "private")),
        ),
    ]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/go")])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("disallows /private/page"));
    assert_eq!(server.hits("/private/page"), 0);
}

#[test]
fn cross_host_redirect_checks_the_target_hosts_robots_txt() {
    let target = TestServer::start(vec![
        (
            "/robots.txt",
            always(|| Reply::ok("text/plain", "User-agent: *\nDisallow: /\n")),
        ),
        ("/page", always(|| Reply::ok("text/plain", "elsewhere"))),
    ]);
    let location = target.url("/page");
    let origin = TestServer::start(vec![(
        "/go",
        always(move || Reply::status(301).header("Location", &location)),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &origin.url("/go")])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("robots.txt"));
    assert_eq!(target.hits("/page"), 0);
    assert_eq!(target.hits("/robots.txt"), 1);
}

#[test]
fn redirect_to_an_allowed_path_is_followed_and_cached_under_the_original_url() {
    let server = site("User-agent: *\nDisallow: /private\n");
    let moved = TestServer::start(vec![(
        "/old",
        always({
            let to = server.url("/public");
            move || Reply::status(308).header("Location", &to)
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", &moved.url("/old")])
        .assert()
        .success()
        .stdout("public page");
    noted(&db)
        .args(["get", &moved.url("/old")])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: hit"));
    assert_eq!(moved.hits("/old"), 1);
}

#[test]
fn unreachable_robots_txt_serves_a_stale_page() {
    let server = TestServer::start(vec![
        (
            "/robots.txt",
            common::counting(|n| {
                if n == 1 {
                    Reply::status(404)
                } else {
                    Reply::status(503)
                }
            }),
        ),
        ("/page", always(|| Reply::ok("text/plain", "cached copy"))),
    ]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", "--ttl", "0s", &server.url("/page")])
        .assert()
        .success();
    rusqlite::Connection::open(&db)
        .unwrap()
        .execute("DELETE FROM robots", [])
        .unwrap();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success()
        .stdout("cached copy")
        .stderr(
            predicate::str::contains("couldn't check robots.txt")
                .and(predicate::str::contains("noted: stale")),
        );
    assert_eq!(server.hits("/page"), 1);
}

#[test]
fn agents_sharing_a_cache_get_their_own_robots_verdict() {
    let server = TestServer::start(vec![
        (
            "/robots.txt",
            always(|| {
                Reply::ok(
                    "text/plain",
                    "User-agent: AgentA\nDisallow: /p\n\nUser-agent: *\nAllow: /\n",
                )
            }),
        ),
        ("/p", always(|| Reply::ok("text/plain", "page"))),
    ]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .env("NOTED_USER_AGENT", "AgentA/1.0")
        .args(["get", &server.url("/p")])
        .assert()
        .code(1);
    noted(&db)
        .env("NOTED_USER_AGENT", "AgentB/1.0")
        .args(["get", &server.url("/p")])
        .assert()
        .success()
        .stdout("page");
    assert_eq!(server.hits("/robots.txt"), 1);
}
