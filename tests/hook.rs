mod common;

use common::noted;
use predicates::prelude::*;
use tempfile::TempDir;

#[test]
fn hook_always_exits_zero() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .arg("hook")
        .write_stdin("{not json")
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hook_rewrites_curl_from_stdin() {
    let dir = TempDir::new().unwrap();
    noted(&dir.path().join("c.db"))
        .arg("hook")
        .write_stdin(r#"{"tool_name":"Bash","tool_input":{"command":"curl -s https://a.io/x"}}"#)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            r#""command":"noted get https://a.io/x""#,
        ));
}

#[test]
fn hook_respects_allow_webfetch() {
    let dir = TempDir::new().unwrap();
    let input = r#"{"tool_name":"WebFetch","tool_input":{"url":"https://a.io/"}}"#;
    noted(&dir.path().join("c.db"))
        .arg("hook")
        .write_stdin(input)
        .assert()
        .success()
        .stdout(predicate::str::contains("deny"));
    noted(&dir.path().join("c.db"))
        .arg("hook")
        .env("NOTED_ALLOW_WEBFETCH", "1")
        .write_stdin(input)
        .assert()
        .success()
        .stdout("");
}
