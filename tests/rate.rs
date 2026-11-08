mod common;

use std::path::Path;
use std::time::Duration;

use common::{Reply, TestServer, always, noted, noted_process};
use predicates::prelude::*;
use tempfile::TempDir;

fn pages(robots: &'static str) -> TestServer {
    TestServer::start(vec![
        (
            "/robots.txt",
            always(move || Reply::ok("text/plain", robots)),
        ),
        ("/p0", always(|| Reply::ok("text/plain", "zero"))),
        ("/p1", always(|| Reply::ok("text/plain", "one"))),
        ("/p2", always(|| Reply::ok("text/plain", "two"))),
        (
            "/hop",
            always(|| Reply::status(302).header("Location", "/p2")),
        ),
    ])
}

fn set_gap(db: &Path, gap: &str) {
    noted(db)
        .args(["config", "rate", "127.0.0.1", gap])
        .assert()
        .success();
}

fn span(server: &TestServer) -> Duration {
    let times = server.arrival_times("/p");
    times[times.len() - 1] - times[0]
}

#[test]
fn a_second_request_waits_for_the_gap() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "2s");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", &server.url("/p1")])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: waiting").and(predicate::str::contains("gap 2s")));
    assert!(
        span(&server) >= Duration::from_millis(1950),
        "{:?}",
        span(&server)
    );
}

#[test]
fn a_wait_past_max_wait_exits_4_without_fetching() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "1m");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--max-wait", "1s", &server.url("/p1")])
        .assert()
        .code(4)
        .stdout("")
        .stderr(predicate::str::contains(
            "127.0.0.1 is rate limited, retry after",
        ));
    assert_eq!(server.hits("/p1"), 0);
}

#[test]
fn a_rate_limited_stale_page_is_served_unless_refreshing() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "1m");
    noted(&db)
        .args(["get", "--ttl", "0s", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--max-wait", "1s", &server.url("/p0")])
        .assert()
        .success()
        .stdout("zero")
        .stderr(
            predicate::str::contains("rate limited, retry after")
                .and(predicate::str::contains("serving the stale copy")),
        );
    noted(&db)
        .args(["get", "--refresh", "--max-wait", "1s", &server.url("/p0")])
        .assert()
        .code(4);
    assert_eq!(server.hits("/p0"), 1);
}

#[test]
fn redirect_hops_share_one_max_wait() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "1s");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--max-wait", "1500ms", &server.url("/hop")])
        .assert()
        .code(4);
    assert_eq!(server.hits("/p2"), 0);
}

#[test]
fn crawl_delay_from_robots_txt_spaces_requests() {
    let server = pages("User-agent: *\nCrawl-delay: 2\n");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", &server.url("/p1")])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: waiting"));
    assert!(
        span(&server) >= Duration::from_millis(1950),
        "{:?}",
        span(&server)
    );
}

#[test]
fn a_huge_crawl_delay_is_capped_at_a_minute() {
    let server = pages("User-agent: *\nCrawl-delay: 10000000000\n");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--max-wait", "1s", &server.url("/p1")])
        .assert()
        .code(4)
        .stderr(
            predicate::str::contains("retry after")
                .and(predicate::str::contains("year").not())
                .and(predicate::str::contains("day").not())
                .and(predicate::str::contains("h ").not()),
        );
}

#[test]
fn cache_hits_never_wait() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "1m");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--max-wait", "0s", &server.url("/p0")])
        .assert()
        .success()
        .stderr(
            predicate::str::contains("noted: hit").and(predicate::str::contains("waiting").not()),
        );
}

#[test]
fn concurrent_agents_take_turns() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "500ms");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    let children: Vec<_> = ["/p1", "/p2", "/p0?again"]
        .iter()
        .map(|path| {
            noted_process(&db)
                .args(["get", "-q", "--refresh", &server.url(path)])
                .stdout(std::process::Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    assert_eq!(server.arrival_times("/p").len(), 4);
    assert!(
        span(&server) >= Duration::from_millis(1450),
        "{:?}",
        span(&server)
    );
}
