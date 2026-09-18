mod common;

use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{Handler, Reply, Seen, TestServer, always, noted};
use predicates::prelude::*;
use tempfile::TempDir;

fn page_server() -> TestServer {
    TestServer::start(vec![(
        "/page",
        always(|| Reply::ok("text/plain", "hello page")),
    )])
}

#[test]
fn miss_fetches_and_prints_body() {
    let dir = TempDir::new().unwrap();
    let server = page_server();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/page")])
        .assert()
        .success()
        .stdout("hello page")
        .stderr(predicate::str::contains("noted: miss"));
    assert_eq!(server.hits("/page"), 1);
}

#[test]
fn fresh_hit_skips_network() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success()
        .stdout("hello page")
        .stderr(predicate::str::contains("noted: hit"));
    assert_eq!(server.hits("/page"), 1);
}

#[test]
fn fragment_shares_entry() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    noted(&db)
        .args(["get", &format!("{}#Drops", server.url("/page"))])
        .assert()
        .success();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success();
    assert_eq!(server.hits("/page"), 1);
}

#[test]
fn binary_body_exact() {
    let png = vec![0x89, b'P', b'N', b'G', 0, 0xff];
    let body = png.clone();
    let server = TestServer::start(vec![(
        "/img",
        always(move || Reply::ok("image/png", body.clone())),
    )]);
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/img")])
        .assert()
        .success()
        .stdout(png);
}

#[test]
fn quiet_suppresses_status_line() {
    let dir = TempDir::new().unwrap();
    let server = page_server();
    noted(&dir.path().join("c.db"))
        .args(["get", "--quiet", &server.url("/page")])
        .assert()
        .success()
        .stderr("");
}

#[test]
fn rejects_non_http_scheme() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "file:///etc/passwd"])
        .assert()
        .failure();
}

#[test]
fn sends_user_agent() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success();
    let agent = server.last_header("/page", "User-Agent").unwrap();
    assert!(agent.starts_with("noted/"), "{agent}");

    noted(&db)
        .env("NOTED_CONTACT", "me@x")
        .args(["get", "--refresh", &server.url("/page")])
        .assert()
        .success();
    let agent = server.last_header("/page", "User-Agent").unwrap();
    assert!(
        agent.starts_with("noted/") && agent.ends_with("(me@x)"),
        "{agent}"
    );
}

#[test]
fn concurrent_processes_succeed() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let routes = (0..8)
        .map(|i| {
            let path: &'static str = Box::leak(format!("/p{i}").into_boxed_str());
            (path, always(move || Reply::ok("text/plain", path)))
        })
        .collect();
    let server = TestServer::start(routes);
    let children: Vec<_> = (0..8)
        .map(|i| {
            std::process::Command::new(env!("CARGO_BIN_EXE_noted"))
                .env("NOTED_DB", &db)
                .args(["get", &server.url(&format!("/p{i}"))])
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{stderr}");
        assert!(!stderr.contains("locked"), "{stderr}");
    }
}

fn counting(reply: impl Fn(usize) -> Reply + Send + Sync + 'static) -> Handler {
    let calls = AtomicUsize::new(0);
    Box::new(move |_| reply(calls.fetch_add(1, Ordering::SeqCst) + 1))
}

#[test]
fn stale_with_etag_revalidates_304() {
    let server = TestServer::start(vec![(
        "/e",
        Box::new(|seen: &Seen| {
            if seen.header("If-None-Match") == Some("\"v1\"") {
                Reply::status(304)
            } else {
                Reply::ok("text/plain", "etag body").header("ETag", "\"v1\"")
            }
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/e")];
    noted(&db).args(args).assert().success();
    noted(&db)
        .args(args)
        .assert()
        .success()
        .stdout("etag body")
        .stderr(predicate::str::contains("noted: revalidated"));
    assert_eq!(server.hits("/e"), 2);
    assert_eq!(
        server.last_header("/e", "If-None-Match").as_deref(),
        Some("\"v1\"")
    );
}

#[test]
fn stale_uses_last_modified() {
    const STAMP: &str = "Wed, 01 Jan 2026 00:00:00 GMT";
    let server = TestServer::start(vec![(
        "/lm",
        Box::new(|seen: &Seen| {
            if seen.header("If-Modified-Since") == Some(STAMP) {
                Reply::status(304)
            } else {
                Reply::ok("text/plain", "lm body").header("Last-Modified", STAMP)
            }
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/lm")];
    noted(&db).args(args).assert().success();
    noted(&db)
        .args(args)
        .assert()
        .success()
        .stdout("lm body")
        .stderr(predicate::str::contains("noted: revalidated"));
}

#[test]
fn stale_without_validators_refetches() {
    let server = TestServer::start(vec![(
        "/v",
        counting(|n| Reply::ok("text/plain", format!("v{n}"))),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/v")];
    noted(&db).args(args).assert().success().stdout("v1");
    noted(&db)
        .args(args)
        .assert()
        .success()
        .stdout("v2")
        .stderr(predicate::str::contains("noted: refreshed"));
}

#[test]
fn refresh_flag_bypasses_fresh() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    noted(&db)
        .args(["get", "--ttl", "1d", &server.url("/page")])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--refresh", &server.url("/page")])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: refreshed"));
    assert_eq!(server.hits("/page"), 2);
}

#[test]
fn explicit_ttl_overwrites_stored() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    let url = server.url("/page");
    noted(&db)
        .args(["get", "--ttl", "0s", &url])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--ttl", "1d", &url])
        .assert()
        .success();
    noted(&db)
        .args(["get", &url])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: hit").and(predicate::str::contains("ttl 1d")));
    assert_eq!(server.hits("/page"), 1);
}

#[test]
fn server_down_serves_stale() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    let url = server.url("/page");
    noted(&db)
        .args(["get", "--ttl", "0s", &url])
        .assert()
        .success();
    server.stop();
    noted(&db)
        .args(["get", &url])
        .assert()
        .success()
        .stdout("hello page")
        .stderr(predicate::str::contains("noted: stale"));
}

#[test]
fn server_500_serves_stale() {
    let server = TestServer::start(vec![(
        "/flaky",
        counting(|n| {
            if n == 1 {
                Reply::ok("text/plain", "good")
            } else {
                Reply::status(500).body("boom")
            }
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/flaky")];
    noted(&db).args(args).assert().success();
    noted(&db)
        .args(args)
        .assert()
        .success()
        .stdout("good")
        .stderr(predicate::str::contains("noted: stale"));
}

#[test]
fn not_found_is_not_cached() {
    let server = TestServer::start(vec![(
        "/gone",
        always(|| Reply::status(404).body("missing")),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    for _ in 0..2 {
        noted(&db)
            .args(["get", &server.url("/gone")])
            .assert()
            .code(1)
            .stdout("missing")
            .stderr(predicate::str::contains("noted: HTTP 404"));
    }
    assert_eq!(server.hits("/gone"), 2);
}

#[test]
fn hung_server_times_out_to_stale() {
    let server = TestServer::start(vec![(
        "/slow",
        counting(|n| {
            if n > 1 {
                std::thread::sleep(std::time::Duration::from_secs(3));
            }
            Reply::ok("text/plain", "cached")
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/slow")];
    noted(&db).args(args).assert().success();
    noted(&db)
        .env("NOTED_TIMEOUT", "1s")
        .args(args)
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success()
        .stdout("cached")
        .stderr(predicate::str::contains("noted: stale"));
}

#[test]
fn bad_url_creates_no_database() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["get", "file:///etc/passwd"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("only http and https"));
    assert!(!db.exists());
}
