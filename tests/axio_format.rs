//! Reading Axio session logs into the normalised model.

use std::path::{Path, PathBuf};

use agent_replay::format::{self, SessionSource};
use agent_replay::model::EventKind;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn detects_an_axio_session_from_its_contents() {
    let path = fixture("axio-session.jsonl");
    assert_eq!(format::detect(&path).unwrap(), SessionSource::Axio);
}

#[test]
fn reads_the_session_header() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    assert_eq!(session.source, SessionSource::Axio);
    assert_eq!(session.cwd, Some(PathBuf::from("/work/demo-app")));
    assert_eq!(session.id, "axio-session");
}

#[test]
fn reads_the_events_in_order() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    let kinds: Vec<&str> = session.events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(
        kinds,
        vec![
            "user",
            "assistant",
            "tool",
            "result",
            "notice",
            "tool",
            "result",
            "assistant",
        ]
    );
}

#[test]
fn reads_a_tool_call_with_its_input() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    let EventKind::ToolCall { name, input } = &session.events[2].kind else {
        panic!("expected a tool call, got {:?}", session.events[2].kind);
    };
    assert_eq!(name, "Read");
    assert_eq!(input["file_path"], "/work/demo-app/src/routes.rs");
}

#[test]
fn reads_a_tool_result() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    let EventKind::ToolResult { output, is_error } = &session.events[3].kind else {
        panic!("expected a tool result, got {:?}", session.events[3].kind);
    };
    assert_eq!(output, "pub fn routes() {}\n");
    assert!(!is_error);
}

#[test]
fn marks_a_failed_tool_result() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    let EventKind::ToolResult { output, is_error } = &session.events[6].kind else {
        panic!("expected a tool result, got {:?}", session.events[6].kind);
    };
    assert!(is_error);
    assert!(output.contains("no such command"));
}

#[test]
fn links_a_tool_call_to_its_result() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    assert_eq!(session.events[2].tool_call_id.as_deref(), Some("call_01"));
    assert_eq!(session.events[3].tool_call_id.as_deref(), Some("call_01"));
}

#[test]
fn describes_an_opaque_block_without_interpreting_it() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    let EventKind::Notice { text } = &session.events[4].kind else {
        panic!("expected a notice, got {:?}", session.events[4].kind);
    };
    assert!(text.contains("anthropic"));
    assert!(text.contains("opaque"));
}

#[test]
fn reports_no_usage_because_the_format_records_none() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    assert_eq!(session.total_usage().total(), 0);
}

#[test]
fn counts_tool_calls() {
    let session = format::read(&fixture("axio-session.jsonl")).unwrap();
    let counts = session.tool_counts();
    assert_eq!(counts.get("Read"), Some(&1));
    assert_eq!(counts.get("Bash"), Some(&1));
}
