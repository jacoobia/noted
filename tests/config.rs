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
