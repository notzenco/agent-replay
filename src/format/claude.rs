//! Reads Claude Code session logs.
//!
//! A session is an append-only JSONL file under
//! `~/.claude/projects/<project>/<session>.jsonl`. Every line is a JSON
//! object discriminated by a top-level `type`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::format::{parse_timestamp, string_field, u64_field};
use crate::model::{Event, EventKind, Session, SessionSource, Usage};

impl super::SessionReader for ClaudeCode {
    fn read(path: &Path) -> Result<Session> {
        read_session(path)
    }

    fn detect(path: &Path) -> bool {
        detect_path(path)
    }
}

/// Marker type so `SessionReader` can be implemented for a unit-like value.
pub struct ClaudeCode;

/// The session file name Claude Code writes.
const SESSION_DIR: &str = ".claude/projects";

/// Read a Claude Code session log.
pub fn read_session(path: &Path) -> Result<Session> {
    let file =
        File::open(path).with_context(|| format!("opening session log {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut session = Session {
        id: String::new(),
        source: SessionSource::ClaudeCode,
        cwd: None,
        model: None,
        started_at: None,
        events: Vec::new(),
    };
    let mut malformed = 0usize;
    // The event carrying each assistant message's usage, by message id.
    let mut usage_at: HashMap<String, usize> = HashMap::new();

    for (line_no, line) in reader.lines().enumerate() {
        let line =
            line.with_context(|| format!("reading line {} of {}", line_no + 1, path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(_) => {
                malformed += 1;
                continue;
            }
        };
        apply(&mut session, &value, &mut usage_at);
    }

    if session.events.is_empty() {
        bail!("{} holds no readable Claude Code events", path.display());
    }
    if malformed > 0 {
        eprintln!(
            "agent-replay: skipped {malformed} unparsable line(s) in {}",
            path.display()
        );
    }
    Ok(session)
}

/// Whether a path looks like a Claude Code session log.
pub fn detect_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if !text.contains(SESSION_DIR) {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("jsonl") | Some("json")
    )
}

/// Fold one JSONL record into the session.
fn apply(session: &mut Session, value: &Value, usage_at: &mut HashMap<String, usize>) {
    if session.id.is_empty()
        && let Some(id) =
            string_field(value, "sessionId").or_else(|| string_field(value, "session_id"))
    {
        session.id = id;
    }
    if session.cwd.is_none()
        && let Some(cwd) = string_field(value, "cwd")
    {
        session.cwd = Some(cwd.into());
    }

    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "user" => apply_user(session, value),
        "assistant" => apply_assistant(session, value, usage_at),
        "system" => apply_system(session, value),
        _ => {}
    }
}

/// A user record: either a prompt or the results of tool calls.
fn apply_user(session: &mut Session, value: &Value) {
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp);
    let is_meta = value
        .get("isMeta")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(content) = value.get("message").and_then(|m| m.get("content")) else {
        return;
    };

    match content {
        Value::String(text) => {
            push(session, EventKind::User { text: text.clone() }, ts);
        }
        Value::Array(blocks) => {
            for block in blocks {
                let block_type = block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match block_type {
                    "text" => {
                        let text = string_field(block, "text").unwrap_or_default();
                        if text.trim().is_empty() {
                            continue;
                        }
                        if is_meta {
                            push(session, EventKind::Notice { text }, ts);
                        } else {
                            push(session, EventKind::User { text }, ts);
                        }
                    }
                    "tool_result" => {
                        let output = render_tool_result(block.get("content"));
                        let is_error = block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        push(session, EventKind::ToolResult { output, is_error }, ts);
                        if let Some(id) = string_field(block, "tool_use_id")
                            && let Some(last) = session.events.last_mut()
                        {
                            last.tool_call_id = Some(id);
                        }
                    }
                    "image" => {
                        push(
                            session,
                            EventKind::Notice {
                                text: "[image attachment]".to_string(),
                            },
                            ts,
                        );
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// An assistant record: text, reasoning and tool calls.
fn apply_assistant(session: &mut Session, value: &Value, usage_at: &mut HashMap<String, usize>) {
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp);
    let message = value.get("message");
    let model = message.and_then(|m| string_field(m, "model"));
    let message_id = message.and_then(|m| string_field(m, "id"));
    let usage = message.and_then(|m| m.get("usage")).map(parse_usage);

    if session.model.is_none() {
        session.model = model.clone();
    }

    let Some(blocks) = message
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
    else {
        return;
    };

    // A streamed message is written as one record per content block, each
    // repeating the usage with the output count still growing. When this
    // message already has an event carrying its usage, the new reading
    // replaces that one instead of being counted again.
    let mut usage = usage;
    if let (Some(latest), Some(id)) = (usage, message_id.as_deref())
        && let Some(index) = usage_at.get(id)
    {
        session.events[*index].usage = Some(latest);
        usage = None;
    }
    for block in blocks {
        let block_type = block
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let kind = match block_type {
            "text" => {
                let text = string_field(block, "text").unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                EventKind::Assistant { text }
            }
            "thinking" => {
                let text = string_field(block, "thinking").unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                EventKind::Reasoning { text }
            }
            "tool_use" => {
                let name = string_field(block, "name").unwrap_or_else(|| "tool".to_string());
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                EventKind::ToolCall { name, input }
            }
            _ => continue,
        };

        let mut event = new_event(kind, ts);
        event.usage = usage.take();
        event.model = model.clone();
        if matches!(event.kind, EventKind::ToolCall { .. }) {
            event.tool_call_id = string_field(block, "id");
        }
        let carries_usage = event.usage.is_some();
        push_event(session, event);
        if carries_usage && let Some(id) = &message_id {
            usage_at.insert(id.clone(), session.events.len() - 1);
        }
    }
}

/// A system record: turn timing and hook outcomes.
fn apply_system(session: &mut Session, value: &Value) {
    let ts = value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp);
    let subtype = value
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or_default();

    match subtype {
        "turn_duration" => {
            let duration_ms = u64_field(value, "durationMs");
            let text = match duration_ms {
                Some(ms) => format!("turn took {}.{}s", ms / 1000, (ms % 1000) / 100),
                None => "turn finished".to_string(),
            };
            let mut event = new_event(EventKind::Notice { text }, ts);
            event.duration_ms = duration_ms;
            push_event(session, event);
        }
        "stop_hook_summary" => {
            let errors = value
                .get("hookErrors")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            if errors > 0 {
                push(
                    session,
                    EventKind::Notice {
                        text: format!("stop hook reported {errors} error(s)"),
                    },
                    ts,
                );
            }
        }
        _ => {}
    }
}

/// Read the usage block of an assistant message.
fn parse_usage(usage: &Value) -> Usage {
    Usage {
        input_tokens: u64_field(usage, "input_tokens").unwrap_or(0),
        output_tokens: u64_field(usage, "output_tokens").unwrap_or(0),
        cache_read_tokens: u64_field(usage, "cache_read_input_tokens").unwrap_or(0),
        cache_creation_tokens: u64_field(usage, "cache_creation_input_tokens").unwrap_or(0),
        reasoning_tokens: usage
            .get("output_tokens_details")
            .and_then(|d| u64_field(d, "thinking_tokens"))
            .unwrap_or(0),
    }
}

/// Render the `content` of a tool result, which is a string or a block array.
fn render_tool_result(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => {
            let mut parts = Vec::new();
            for block in blocks {
                match block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                {
                    "text" => parts.push(string_field(block, "text").unwrap_or_default()),
                    "image" => parts.push("[image]".to_string()),
                    _ => {}
                }
            }
            parts.join("\n")
        }
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// Build an event with the session's current index.
fn new_event(kind: EventKind, ts: Option<time::OffsetDateTime>) -> Event {
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
fn push(session: &mut Session, kind: EventKind, ts: Option<time::OffsetDateTime>) {
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
