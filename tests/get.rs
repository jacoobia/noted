mod common;

use std::process::Stdio;

use common::{
    Reply, Seen, TEST_AGENT, TestServer, always, answer_once_then_reset, counting, noted,
};
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
fn sends_the_configured_user_agent() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success();
    assert_eq!(
        server.last_header("/page", "User-Agent").unwrap(),
        TEST_AGENT
    );

    let saved = "Claude-Code/2.1 (+mailto:jo@example.com)";
    noted(&db)
        .args(["config", "user-agent", saved])
        .assert()
        .success();
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .args(["get", "--refresh", &server.url("/page")])
        .assert()
        .success();
    assert_eq!(server.last_header("/page", "User-Agent").unwrap(), saved);
}

#[test]
fn fetching_without_a_user_agent_asks_for_setup() {
    let dir = TempDir::new().unwrap();
    let server = page_server();
    noted(&dir.path().join("c.db"))
        .env_remove("NOTED_USER_AGENT")
        .args(["get", &server.url("/page")])
        .assert()
        .code(3)
        .stdout("")
        .stderr(
            predicate::str::contains("no User-Agent is set")
                .and(predicate::str::contains("noted config user-agent")),
        );
    assert_eq!(server.hits("/page"), 0);
}

#[test]
fn cache_hits_need_no_user_agent() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let server = page_server();
    noted(&db)
        .args(["get", &server.url("/page")])
        .assert()
        .success();
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .args(["get", &server.url("/page")])
        .assert()
        .success()
        .stdout("hello page");
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
            common::noted_process(&db)
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
    let listing = noted(&db)
        .arg("ls")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert_eq!(String::from_utf8(listing).unwrap().lines().count(), 8);
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
    assert_eq!(
        server.last_header("/e", "If-None-Match").as_deref(),
        Some("\"v1\"")
    );
}

#[test]
fn revalidation_persists_new_ttl() {
    let server = TestServer::start(vec![(
        "/e",
        Box::new(|seen: &Seen| {
            if seen.header("If-None-Match").is_some() {
                Reply::status(304)
            } else {
                Reply::ok("text/plain", "etag body").header("ETag", "\"v1\"")
            }
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let url = server.url("/e");
    noted(&db)
        .args(["get", "--ttl", "0s", &url])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--refresh", "--ttl", "1d", &url])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: revalidated"));
    noted(&db)
        .args(["get", &url])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: hit"));
    assert_eq!(server.hits("/e"), 2);
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
    let url = answer_once_then_reset("hello page");
    noted(&db)
        .args(["get", "--ttl", "0s", &url])
        .assert()
        .success();
    noted(&db)
        .args(["get", &url])
        .assert()
        .success()
        .stdout("hello page")
        .stderr(predicate::str::contains("noted: stale"));
}

#[test]
fn offline_ttl_is_still_saved() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let url = answer_once_then_reset("hello page");
    noted(&db)
        .args(["get", "--ttl", "0s", &url])
        .assert()
        .success();
    noted(&db)
        .args(["get", "--refresh", "--ttl", "7d", &url])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: stale"));
    noted(&db)
        .args(["get", &url])
        .assert()
        .success()
        .stderr(predicate::str::contains("noted: hit").and(predicate::str::contains("ttl 7d")));
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
            .stdout("")
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

#[test]
fn client_error_on_stale_entry_is_reported() {
    let server = TestServer::start(vec![(
        "/later-gone",
        counting(|n| {
            if n == 1 {
                Reply::ok("text/plain", "was here")
            } else {
                Reply::status(404).body("missing")
            }
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/later-gone")];
    noted(&db).args(args).assert().success();
    noted(&db)
        .args(args)
        .assert()
        .code(1)
        .stdout("")
        .stderr(predicate::str::contains("noted: HTTP 404"));
}

#[test]
fn oversized_body_is_an_error_even_when_stale() {
    let server = TestServer::start(vec![(
        "/big",
        counting(|n| {
            let size = if n == 1 { 10 } else { 21 * 1024 * 1024 };
            Reply::ok("application/octet-stream", vec![b'x'; size])
        }),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let args = ["get", "--ttl", "0s", &server.url("/big")];
    noted(&db).args(args).assert().success();
    noted(&db)
        .args(args)
        .assert()
        .failure()
        .stdout("")
        .stderr(predicate::str::contains("limit"));
}

#[test]
fn status_line_shows_the_stored_size() {
    let dir = TempDir::new().unwrap();
    let server = page_server();
    noted(&dir.path().join("c.db"))
        .args(["get", &server.url("/page")])
        .assert()
        .success()
        .stderr(predicate::str::contains("200 text/plain · 10B"));
}

#[test]
fn help_explains_ttl_offline_and_exit_codes() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["get", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("counts from the last time")
                .and(predicate::str::contains("offline"))
                .and(predicate::str::contains("Exit codes")),
        );
    noted(&dir.path().join("c.db"))
        .args(["hook", "--help"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("settings.json").and(predicate::str::contains("WebFetch")),
        );
}
