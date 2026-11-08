mod common;

use common::noted;
use predicates::prelude::*;
use tempfile::TempDir;

const AGENT: &str = "Claude-Code/2.1 (+mailto:jo@example.com)";

#[test]
fn saved_user_agent_round_trips() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .args(["config", "user-agent", AGENT])
        .assert()
        .success();
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .args(["config", "user-agent"])
        .assert()
        .success()
        .stdout(format!("{AGENT}\n"));
}

#[test]
fn env_beats_saved_user_agent() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["config", "user-agent", AGENT])
        .assert()
        .success();
    noted(&db)
        .env("NOTED_USER_AGENT", "Other/2.0")
        .args(["config", "user-agent"])
        .assert()
        .success()
        .stdout("Other/2.0\n");
}

#[test]
fn unset_user_agent_says_so() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .env_remove("NOTED_USER_AGENT")
        .args(["config", "user-agent"])
        .assert()
        .code(3)
        .stdout("")
        .stderr(predicate::str::contains("not set"));
}

#[test]
fn invalid_user_agents_are_usage_errors() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let too_long = "a".repeat(257);
    for value in [
        "",
        "   ",
        "bad\u{7}agent",
        "line\r\nInjected: 1",
        "agént/1",
        too_long.as_str(),
    ] {
        noted(&db)
            .args(["config", "user-agent", value])
            .assert()
            .code(64);
    }
}

#[test]
fn bad_env_user_agent_is_a_usage_error_before_any_request() {
    let server = common::TestServer::start(vec![(
        "/page",
        common::always(|| common::Reply::ok("text/plain", "ok")),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    for value in ["line\r\nInjected: 1", "agént/1", "/no-token"] {
        noted(&db)
            .env("NOTED_USER_AGENT", value)
            .args(["get", &server.url("/page")])
            .assert()
            .code(64)
            .stderr(predicate::str::contains("NOTED_USER_AGENT"));
    }
    assert_eq!(server.hits("/page"), 0);
}

#[test]
fn bad_saved_user_agent_asks_for_setup_again() {
    let server = common::TestServer::start(vec![(
        "/page",
        common::always(|| common::Reply::ok("text/plain", "ok")),
    )]);
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    let config = common::config_dir(&db);
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join("user-agent"), "bad\u{7}agent\n").unwrap();
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .args(["get", &server.url("/page")])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("noted config user-agent"));
    noted(&db)
        .env_remove("NOTED_USER_AGENT")
        .arg("stats")
        .assert()
        .success()
        .stdout(predicate::str::contains("user agent: invalid"));
    assert_eq!(server.hits("/page"), 0);
}

#[test]
fn rate_overrides_can_be_set_listed_and_removed() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("c.db");
    noted(&db)
        .args(["config", "rate", "Wiki.Example.org", "2m"])
        .assert()
        .success();
    noted(&db)
        .args(["config", "rate"])
        .assert()
        .success()
        .stdout("wiki.example.org 2m\n");
    noted(&db)
        .args(["config", "rate", "wiki.example.org", "0s"])
        .assert()
        .success();
    noted(&db)
        .args(["config", "rate"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn rate_needs_a_real_host() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .args(["config", "rate", "not a host", "1s"])
        .assert()
        .code(64);
}
