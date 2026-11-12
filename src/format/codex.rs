//! Reads Codex rollout logs.
//!
//! A rollout is an append-only JSONL file under `~/.codex/sessions/`. Every
//! line is `{ "timestamp": …, "type": …, "payload": { … } }`.
//!
//! Codex writes the same prose twice: once as an `event_msg` for the UI and
//! once as a `response_item` for the model. The reader keeps the `event_msg`
//! copy and drops the `response_item` duplicate.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use time::OffsetDateTime;

use crate::format::{parse_timestamp, string_field, u64_field};
use crate::model::{Event, EventKind, Session, SessionSource, Usage};

impl super::SessionReader for Codex {
    fn read(path: &Path) -> Result<Session> {
        read_session(path)
    }

    fn detect(path: &Path) -> bool {
        detect_path(path)
    }
}

/// The directory Codex writes rollouts to.
const SESSION_DIR: &str = ".codex/sessions";

/// Marker type so `SessionReader` can be implemented for this format.
pub struct Codex;

/// What an `exec_command_end` event says about a command that ran.
#[derive(Debug, Clone)]
struct CommandEnd {
    duration_ms: Option<u64>,
    exit_code: Option<i64>,
}

/// Read a Codex rollout log.
pub fn read_session(path: &Path) -> Result<Session> {
    let file =
        File::open(path).with_context(|| format!("opening session log {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut records = Vec::new();
    let mut malformed = 0usize;
    for line in reader.lines() {
        let line = line.with_context(|| format!("reading {}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => records.push(value),
            Err(_) => malformed += 1,
        }
    }
    if records.is_empty() {
        bail!("{} holds no readable Codex records", path.display());
    }
    if malformed > 0 {
        eprintln!(
            "agent-replay: skipped {malformed} unparsable line(s) in {}",
            path.display()
        );
    }

    let command_ends = collect_command_ends(&records);
    let spoken = collect_spoken_messages(&records);

    let mut session = Session {
        id: String::new(),
        source: SessionSource::Codex,
        cwd: None,
        model: None,
        started_at: None,
        events: Vec::new(),
    };
    let mut previous_usage: Option<Usage> = None;

    for record in &records {
        let ts = record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp);
        let kind = record
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let payload = record.get("payload");

        match kind {
            "session_meta" => {
                let Some(payload) = payload else { continue };
                if session.id.is_empty()
                    && let Some(id) = string_field(payload, "id")
                {
                    session.id = id;
                }
                if session.cwd.is_none()
                    && let Some(cwd) = string_field(payload, "cwd")
                {
                    session.cwd = Some(cwd.into());
                }
            }
            "turn_context" => {
                let Some(payload) = payload else { continue };
                if session.model.is_none()
                    && let Some(model) = string_field(payload, "model")
                {
                    session.model = Some(model);
                }
                if session.cwd.is_none()
                    && let Some(cwd) = string_field(payload, "cwd")
                {
                    session.cwd = Some(cwd.into());
                }
            }
            "response_item" => {
                let Some(payload) = payload else { continue };
                let item_type = payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match item_type {
                    "message" => {
                        let role = payload
                            .get("role")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let text = message_text(payload);
                        if text.trim().is_empty() {
                            continue;
                        }
                        // The user and assistant prose is also written as an
                        // event; keep only one copy of it.
                        if matches!(role, "user" | "assistant") && spoken.contains(&text) {
                            continue;
                        }
                        match role {
                            "user" => push(&mut session, EventKind::User { text }, ts),
                            "assistant" => push(&mut session, EventKind::Assistant { text }, ts),
                            _ => {}
                        }
                    }
                    "reasoning" => {
                        let text = reasoning_text(payload);
                        if text.trim().is_empty() {
                            continue;
                        }
                        push(&mut session, EventKind::Reasoning { text }, ts);
                    }
                    "function_call" => {
                        let name = string_field(payload, "name").unwrap_or_else(|| "tool".into());
                        let arguments = payload
                            .get("arguments")
                            .and_then(Value::as_str)
                            .and_then(|text| serde_json::from_str::<Value>(text).ok())
                            .unwrap_or(Value::Null);
                        let mut event = new_event(
                            EventKind::ToolCall {
                                name,
                                input: arguments,
                            },
                            ts,
                        );
                        event.tool_call_id = string_field(payload, "call_id");
                        push_event(&mut session, event);
                    }
                    "function_call_output" => {
                        let output = string_field(payload, "output").unwrap_or_default();
                        let call_id = string_field(payload, "call_id");
                        let end = call_id.as_deref().and_then(|id| command_ends.get(id));
                        let is_error = end.and_then(|e| e.exit_code).is_some_and(|code| code != 0);
                        let mut event = new_event(EventKind::ToolResult { output, is_error }, ts);
                        event.tool_call_id = call_id;
                        event.duration_ms = end.and_then(|e| e.duration_ms);
                        push_event(&mut session, event);
                    }
                    _ => {}
                }
            }
            "event_msg" => {
                let Some(payload) = payload else { continue };
                let event_type = payload
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match event_type {
                    "user_message" => {
                        let text = string_field(payload, "message").unwrap_or_default();
                        if text.trim().is_empty() {
                            continue;
                        }
                        push(&mut session, EventKind::User { text }, ts);
                    }
                    "agent_message" => {
                        let text = string_field(payload, "message").unwrap_or_default();
                        if text.trim().is_empty() {
                            continue;
                        }
                        push(&mut session, EventKind::Assistant { text }, ts);
                    }
                    "token_count" => {
                        let Some(info) = payload.get("info") else {
                            continue;
                        };
                        let Some(total) = info.get("total_token_usage") else {
                            continue;
                        };
                        let observed = parse_token_usage(total);
                        let delta = observed.difference(previous_usage.unwrap_or_default());
                        previous_usage = Some(observed);
                        attach_usage(&mut session, delta);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    if session.events.is_empty() {
        bail!("{} holds no readable Codex events", path.display());
    }
    Ok(session)
}

/// Whether a path looks like a Codex rollout log.
pub fn detect_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains(SESSION_DIR)
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-"))
}

/// Collect the `exec_command_end` records, keyed by the call they belong to.
fn collect_command_ends(records: &[Value]) -> HashMap<String, CommandEnd> {
    let mut ends = HashMap::new();
    for record in records {
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = record.get("payload") else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("exec_command_end") {
            continue;
        }
        let Some(call_id) = string_field(payload, "call_id") else {
            continue;
        };
        let duration_ms = payload.get("duration").map(|d| {
            let secs = u64_field(d, "secs").unwrap_or(0);
            let nanos = u64_field(d, "nanos").unwrap_or(0);
            secs * 1000 + nanos / 1_000_000
        });
        let exit_code = payload.get("exit_code").and_then(Value::as_i64);
        ends.insert(
            call_id,
            CommandEnd {
                duration_ms,
                exit_code,
            },
        );
    }
    ends
}

/// Collect the prose Codex writes as events, so the duplicate copy in the
/// response items can be recognised and dropped.
fn collect_spoken_messages(records: &[Value]) -> Vec<String> {
    let mut spoken = Vec::new();
    for record in records {
        if record.get("type").and_then(Value::as_str) != Some("event_msg") {
            continue;
        }
        let Some(payload) = record.get("payload") else {
            continue;
        };
        let event_type = payload
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(event_type, "user_message" | "agent_message")
            && let Some(message) = string_field(payload, "message")
        {
            spoken.push(message);
        }
    }
    spoken
}

/// The prose of a `response_item` message.
fn message_text(payload: &Value) -> String {
    let Some(blocks) = payload.get("content").and_then(Value::as_array) else {
        return string_field(payload, "text").unwrap_or_default();
    };
    let mut parts = Vec::new();
    for block in blocks {
        if let Some(text) = string_field(block, "text") {
            parts.push(text);
        }
    }
    parts.join("\n")
}

/// The prose of a `response_item` reasoning record.
fn reasoning_text(payload: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(blocks) = payload.get("summary").and_then(Value::as_array) {
        for block in blocks {
            if let Some(text) = string_field(block, "text") {
                parts.push(text);
            }
        }
    }
    if parts.is_empty()
        && let Some(blocks) = payload.get("content").and_then(Value::as_array)
    {
        for block in blocks {
            if let Some(text) = string_field(block, "text") {
                parts.push(text);
            }
        }
    }
    parts.join("\n")
}

/// Read Codex's cumulative token counters.
fn parse_token_usage(total: &Value) -> Usage {
    let input = u64_field(total, "input_tokens").unwrap_or(0);
    let cached = u64_field(total, "cached_input_tokens").unwrap_or(0);
    Usage {
        // Codex counts cached input inside `input_tokens`, so the non-cached
        // part is pulled out to match how the other formats report it.
        input_tokens: input.saturating_sub(cached),
        output_tokens: u64_field(total, "output_tokens").unwrap_or(0),
        cache_read_tokens: cached,
        cache_creation_tokens: 0,
        reasoning_tokens: u64_field(total, "reasoning_output_tokens").unwrap_or(0),
    }
}

/// Attach a usage reading to the most recent event that has none.
fn attach_usage(session: &mut Session, usage: Usage) {
    if usage == Usage::default() {
        return;
    }
    if let Some(event) = session
        .events
        .iter_mut()
        .rev()
        .find(|e| e.usage.is_none() && !matches!(e.kind, EventKind::User { .. }))
    {
        event.usage = Some(usage);
    }
}

/// Build an event with no timestamp yet filled in.
fn new_event(kind: EventKind, ts: Option<OffsetDateTime>) -> Event {
    Event {
        index: 0,
        kind,
        ts,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    }
}

/// Append an event and stamp it with its position in the session.
fn push(session: &mut Session, kind: EventKind, ts: Option<OffsetDateTime>) {
    push_event(session, new_event(kind, ts));
}

/// Append an event and stamp it with its position in the session.
fn push_event(session: &mut Session, mut event: Event) {
    event.index = session.events.len();
    if session.started_at.is_none() {
        session.started_at = event.ts;
    }
    session.events.push(event);
}
