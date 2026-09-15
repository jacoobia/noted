mod common;

use std::process::Stdio;

use common::{Reply, TestServer, always, noted};
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
