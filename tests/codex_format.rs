//! Reading Codex rollout logs into the normalised model.

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
fn detects_a_codex_rollout_from_its_contents() {
    let path = fixture("codex-session.jsonl");
    assert_eq!(format::detect(&path).unwrap(), SessionSource::Codex);
}

#[test]
fn reads_the_session_header() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    assert_eq!(session.source, SessionSource::Codex);
    assert_eq!(session.id, "019d389d-9fb3-7d32-b636-6050fd1da0ad");
    assert_eq!(session.cwd, Some(PathBuf::from("/work/demo-app")));
    assert_eq!(session.model.as_deref(), Some("gpt-5-codex"));
}

#[test]
fn reads_the_events_in_order() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let kinds: Vec<&str> = session.events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(
        kinds,
        vec![
            "user",
            "reasoning",
            "tool",
            "result",
            "tool",
            "result",
            "assistant"
        ]
    );
}

#[test]
fn does_not_repeat_a_message_that_appears_twice() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let EventKind::User { text } = &session.events[0].kind else {
        panic!("expected a user turn, got {:?}", session.events[0].kind);
    };
    assert_eq!(text, "add a health endpoint");
    let user_turns = session
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::User { .. }))
        .count();
    assert_eq!(user_turns, 1);
}

#[test]
fn reads_a_function_call_with_its_parsed_arguments() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let EventKind::ToolCall { name, input } = &session.events[2].kind else {
        panic!("expected a tool call, got {:?}", session.events[2].kind);
    };
    assert_eq!(name, "shell_command");
    assert_eq!(input["command"], "ls src");
    assert_eq!(input["workdir"], "/work/demo-app");
}

#[test]
fn reads_a_tool_result() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let EventKind::ToolResult { output, is_error } = &session.events[3].kind else {
        panic!("expected a tool result, got {:?}", session.events[3].kind);
    };
    assert!(output.contains("src/routes.rs"));
    assert!(!is_error);
}

#[test]
fn marks_a_failed_command_as_an_error() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let EventKind::ToolResult { output, is_error } = &session.events[5].kind else {
        panic!("expected a tool result, got {:?}", session.events[5].kind);
    };
    assert!(is_error);
    assert!(output.contains("no such command"));
}

#[test]
fn takes_the_duration_from_the_command_end_event() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    assert_eq!(session.events[3].duration_ms, Some(412));
    assert_eq!(session.events[5].duration_ms, Some(2011));
}

#[test]
fn links_a_call_to_its_result() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    assert_eq!(session.events[2].tool_call_id.as_deref(), Some("call_01"));
    assert_eq!(session.events[3].tool_call_id.as_deref(), Some("call_01"));
}

#[test]
fn sums_the_cumulative_token_counts() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let usage = session.total_usage();
    // Codex counts cached input inside `input_tokens`, so the non-cached part
    // is reported separately and the two add back up to Codex's own figure.
    assert_eq!(usage.input_tokens, 13193);
    assert_eq!(usage.cache_read_tokens, 17000);
    assert_eq!(usage.total_input(), 30193);
    assert_eq!(usage.output_tokens, 459);
    assert_eq!(usage.reasoning_tokens, 70);
}

#[test]
fn counts_tool_calls() {
    let session = format::read(&fixture("codex-session.jsonl")).unwrap();
    let counts = session.tool_counts();
    assert_eq!(counts.get("shell_command"), Some(&2));
}

#[test]
fn reads_custom_tool_calls_in_order() {
    let session = format::read(&fixture("codex-custom-tools.jsonl")).unwrap();
    let kinds: Vec<&str> = session.events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(
        kinds,
        vec![
            "user",
            "tool",
            "result",
            "tool",
            "result",
            "tool",
            "result",
            "tool",
            "result",
            "tool",
            "notice",
            "assistant"
        ]
    );
    let tools: Vec<Option<&str>> = session
        .events
        .iter()
        .map(|e| e.tool_name())
        .filter(|name| name.is_some())
        .collect();
    assert_eq!(
        tools,
        vec![
            Some("apply_patch"),
            Some("exec"),
            Some("view_image"),
            Some("tool_search"),
            Some("web_search")
        ]
    );
}

#[test]
fn keeps_a_custom_tool_calls_input_text() {
    let session = format::read(&fixture("codex-custom-tools.jsonl")).unwrap();
    let EventKind::ToolCall { name, input } = &session.events[1].kind else {
        panic!("expected a tool call, got {:?}", session.events[1].kind);
    };
    assert_eq!(name, "apply_patch");
    assert!(
        input["input"]
            .as_str()
            .unwrap()
            .contains("*** Update File: /work/demo-app/src/routes.rs")
    );
    assert_eq!(session.events[1].tool_call_id.as_deref(), Some("call_p1"));
    assert_eq!(session.events[2].tool_call_id.as_deref(), Some("call_p1"));
}

#[test]
fn reads_a_custom_tool_output_written_as_blocks() {
    let session = format::read(&fixture("codex-custom-tools.jsonl")).unwrap();
    let EventKind::ToolResult { output, is_error } = &session.events[2].kind else {
        panic!("expected a tool result, got {:?}", session.events[2].kind);
    };
    assert!(output.contains("Success. Updated the following files:"));
    assert!(!is_error);
}

#[test]
fn reads_a_function_call_output_written_as_blocks() {
    let session = format::read(&fixture("codex-custom-tools.jsonl")).unwrap();
    let EventKind::ToolResult { output, .. } = &session.events[6].kind else {
        panic!("expected a tool result, got {:?}", session.events[6].kind);
    };
    assert!(output.contains("here is the screenshot"));
    assert!(output.contains("[image]"));
    assert!(!output.contains("base64"));
}

#[test]
fn reads_a_tool_search_round_trip() {
    let session = format::read(&fixture("codex-custom-tools.jsonl")).unwrap();
    let EventKind::ToolCall { name, input } = &session.events[7].kind else {
        panic!("expected a tool call, got {:?}", session.events[7].kind);
    };
    assert_eq!(name, "tool_search");
    assert_eq!(input["query"], "create a branch");
    let EventKind::ToolResult { output, .. } = &session.events[8].kind else {
        panic!("expected a tool result, got {:?}", session.events[8].kind);
    };
    assert!(output.contains("create_branch"));
    assert_eq!(session.events[8].tool_call_id.as_deref(), Some("call_s1"));
}

#[test]
fn shows_a_generated_image_as_a_notice_without_the_image_data() {
    let session = format::read(&fixture("codex-custom-tools.jsonl")).unwrap();
    let EventKind::Notice { text } = &session.events[10].kind else {
        panic!("expected a notice, got {:?}", session.events[10].kind);
    };
    assert!(text.contains("image generated"));
    assert!(text.contains("a small diagram of the routes"));
}
