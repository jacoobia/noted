mod common;

use std::path::Path;
use std::time::{Duration, Instant};

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
    ])
}

fn set_gap(db: &Path, gap: &str) {
    noted(db)
        .args(["config", "rate", "127.0.0.1", gap])
        .assert()
        .success();
}

#[test]
fn a_second_request_waits_for_the_gap() {
    let server = pages("");
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    set_gap(&db, "1s");
    noted(&db)
        .args(["get", &server.url("/p0")])
        .assert()
        .success();
    let started = Instant::now();
    noted(&db)
        .args(["get", &server.url("/p1")])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: waiting").and(predicate::str::contains("gap 1s")));
    assert!(
        started.elapsed() >= Duration::from_millis(700),
        "{:?}",
        started.elapsed()
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
fn crawl_delay_from_robots_txt_spaces_requests() {
    let server = pages("User-agent: *\nCrawl-delay: 1\n");
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
    let times = server.arrival_times("/p");
    assert_eq!(times.len(), 4);
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            gap >= Duration::from_millis(300),
            "requests only {gap:?} apart"
        );
    }
}
