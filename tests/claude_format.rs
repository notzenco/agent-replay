//! Reading Claude Code session logs into the normalised model.

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
fn detects_a_claude_code_session_from_its_contents() {
    let path = fixture("claude-session.jsonl");
    assert_eq!(format::detect(&path).unwrap(), SessionSource::ClaudeCode);
}

#[test]
fn reads_the_session_header() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    assert_eq!(session.source, SessionSource::ClaudeCode);
    assert_eq!(session.id, "claude-fixture-0001");
    assert_eq!(session.cwd, Some(PathBuf::from("/work/demo-app")));
    assert_eq!(session.model.as_deref(), Some("claude-sonnet-4-5"));
}

#[test]
fn reads_the_events_in_order() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let kinds: Vec<&str> = session.events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(
        kinds,
        vec![
            "user",
            "assistant",
            "tool",
            "result",
            "tool",
            "result",
            "tool",
            "result",
            "assistant",
            "notice",
        ]
    );
}

#[test]
fn reads_the_user_prompt() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let EventKind::User { text } = &session.events[0].kind else {
        panic!("expected a user turn, got {:?}", session.events[0].kind);
    };
    assert_eq!(text, "add a health endpoint");
}

#[test]
fn reads_tool_calls_with_their_input() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let EventKind::ToolCall { name, input } = &session.events[4].kind else {
        panic!("expected a tool call, got {:?}", session.events[4].kind);
    };
    assert_eq!(name, "Edit");
    assert_eq!(input["file_path"], "/work/demo-app/src/routes.rs");
    assert_eq!(input["replace_all"], false);
    assert!(
        input["old_string"]
            .as_str()
            .unwrap()
            .contains("pub fn routes()")
    );
}

#[test]
fn marks_a_failed_tool_result() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let EventKind::ToolResult { output, is_error } = &session.events[7].kind else {
        panic!("expected a tool result, got {:?}", session.events[7].kind);
    };
    assert!(is_error);
    assert!(output.contains("no such command"));
}

#[test]
fn links_a_tool_call_to_its_result() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    assert_eq!(session.events[2].tool_call_id.as_deref(), Some("toolu_01a"));
    assert_eq!(session.events[3].tool_call_id.as_deref(), Some("toolu_01a"));
}

#[test]
fn counts_usage_once_per_assistant_message() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let usage = session.total_usage();
    assert_eq!(usage.input_tokens, 750);
    assert_eq!(usage.output_tokens, 193);
    assert_eq!(usage.cache_read_tokens, 13700);
    assert_eq!(usage.cache_creation_tokens, 1500);
}

#[test]
fn counts_usage_once_for_a_message_split_over_lines() {
    // Claude Code writes one line per content block of a streamed message,
    // repeating the usage on each line with the output count still growing.
    // Only the last reading is the message's usage.
    let session = format::read(&fixture("claude-split-message.jsonl")).unwrap();
    let usage = session.total_usage();
    assert_eq!(usage.input_tokens, 1200);
    assert_eq!(usage.output_tokens, 110);
    assert_eq!(usage.cache_read_tokens, 500);
    assert_eq!(usage.reasoning_tokens, 7);
    let kinds: Vec<&str> = session.events.iter().map(|e| e.kind.name()).collect();
    assert_eq!(
        kinds,
        vec![
            "user",
            "assistant",
            "tool",
            "tool",
            "result",
            "result",
            "assistant",
            "notice",
            "user"
        ]
    );
}

#[test]
fn shows_a_user_image_block_as_a_notice() {
    let session = format::read(&fixture("claude-split-message.jsonl")).unwrap();
    let last_two: Vec<&EventKind> = session
        .events
        .iter()
        .rev()
        .take(2)
        .map(|e| &e.kind)
        .collect();
    let EventKind::Notice { text } = last_two[1] else {
        panic!("expected a notice, got {:?}", last_two[1]);
    };
    assert_eq!(text, "[image attachment]");
    assert!(matches!(last_two[0], EventKind::User { .. }));
}

#[test]
fn counts_tool_calls_and_files_touched() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let counts = session.tool_counts();
    assert_eq!(counts.get("Read"), Some(&1));
    assert_eq!(counts.get("Edit"), Some(&1));
    assert_eq!(counts.get("Bash"), Some(&1));
    assert_eq!(
        session.files_touched(),
        vec!["/work/demo-app/src/routes.rs".to_string()]
    );
}

#[test]
fn measures_the_wall_clock_duration() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let duration = session.duration().unwrap();
    assert_eq!(duration.as_millis(), 9100);
}

#[test]
fn records_turn_timing_as_a_notice() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let EventKind::Notice { text } = &session.events[9].kind else {
        panic!("expected a notice, got {:?}", session.events[9].kind);
    };
    assert_eq!(text, "turn took 9.1s");
    assert_eq!(session.events[9].duration_ms, Some(9100));
}

#[test]
fn rejects_a_file_it_cannot_recognise() {
    let path = fixture("not-a-session.txt");
    let error = format::detect(&path).unwrap_err();
    assert!(error.to_string().contains("could not tell which agent"));
}
