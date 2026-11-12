//! The command line interface.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn agent_replay() -> Command {
    Command::cargo_bin("agent-replay").unwrap()
}

#[test]
fn stats_prints_what_the_session_did() {
    agent_replay()
        .arg("stats")
        .arg(fixture("claude-session.jsonl"))
        .assert()
        .success()
        .stdout(predicate::str::contains("claude-fixture-0001"))
        .stdout(predicate::str::contains("tool calls"))
        .stdout(predicate::str::contains("Edit"))
        .stdout(predicate::str::contains("/work/demo-app/src/routes.rs"))
        .stdout(predicate::str::contains("tokens"));
}

#[test]
fn stats_can_print_json() {
    let output = agent_replay()
        .arg("stats")
        .arg(fixture("codex-session.jsonl"))
        .arg("--json")
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["source"], "codex");
    assert_eq!(value["tool_calls"]["shell_command"], 2);
    assert_eq!(value["tokens"]["output"], 459);
}

#[test]
fn export_writes_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("timeline.html");
    agent_replay()
        .arg("export")
        .arg(fixture("claude-session.jsonl"))
        .arg("-o")
        .arg(&out)
        .assert()
        .success()
        .stdout(predicate::str::contains("redacted"));
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("claude-fixture-0001"));
}

#[test]
fn export_creates_the_directory_it_is_given() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("nested").join("timeline.html");
    agent_replay()
        .arg("export")
        .arg(fixture("claude-session.jsonl"))
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    assert!(out.exists());
}

#[test]
fn export_redacts_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("timeline.html");
    agent_replay()
        .arg("export")
        .arg(fixture("secret-session.jsonl"))
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.contains("[redacted:aws-key-id]"));
    assert!(!html.contains("AKIAIOSFODNN7EXAMPLE"));
}

#[test]
fn export_can_leave_secrets_in() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("timeline.html");
    agent_replay()
        .arg("export")
        .arg(fixture("secret-session.jsonl"))
        .arg("-o")
        .arg(&out)
        .arg("--no-redact")
        .assert()
        .success()
        .stdout(predicate::str::contains("not redacted"));
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(html.contains("AKIAIOSFODNN7EXAMPLE"));
}

#[test]
fn export_makes_no_external_requests() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("timeline.html");
    agent_replay()
        .arg("export")
        .arg(fixture("claude-session.jsonl"))
        .arg("-o")
        .arg(&out)
        .assert()
        .success();
    let html = std::fs::read_to_string(&out).unwrap();
    assert!(!html.contains("http://"));
    assert!(!html.contains("https://"));
    assert!(!html.contains("<script"));
}

#[test]
fn an_unknown_session_is_an_error() {
    agent_replay()
        .arg("stats")
        .arg("no-such-session-id")
        .assert()
        .failure()
        .stderr(predicate::str::contains("no session called"));
}

#[test]
fn a_file_that_is_not_a_session_is_an_error() {
    agent_replay()
        .arg("stats")
        .arg(fixture("not-a-session.txt"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("could not tell which agent"));
}

#[test]
fn list_runs() {
    agent_replay().arg("list").assert().success();
}
