//! Reads Axio session logs.
//!
//! A session is an append-only JSONL file under `~/.axio/agent/sessions/`.
//! The first line is a header; every following line holds one message with a
//! role and a list of content blocks.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use time::OffsetDateTime;

use crate::format::string_field;
use crate::model::{Event, EventKind, Session, SessionSource};

impl super::SessionReader for Axio {
    fn read(path: &Path) -> Result<Session> {
        read_session(path)
    }

    fn detect(path: &Path) -> bool {
        detect_path(path)
    }
}

/// The directory Axio writes sessions to.
const SESSION_DIR: &str = ".axio/agent/sessions";

/// Marker type so `SessionReader` can be implemented for this format.
pub struct Axio;

/// Read an Axio session log.
pub fn read_session(path: &Path) -> Result<Session> {
    let file =
        File::open(path).with_context(|| format!("opening session log {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut session = Session {
        id: path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("session")
            .to_string(),
        source: SessionSource::Axio,
        cwd: None,
        model: None,
        started_at: None,
        events: Vec::new(),
    };
    let mut malformed = 0usize;

    for line in reader.lines() {
        let line = line.with_context(|| format!("reading {}", path.display()))?;
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
        apply(&mut session, &value);
    }

    if session.events.is_empty() {
        bail!("{} holds no readable Axio events", path.display());
    }
    if malformed > 0 {
        eprintln!(
            "agent-replay: skipped {malformed} unparsable line(s) in {}",
            path.display()
        );
    }
    Ok(session)
}

/// Whether a path looks like an Axio session log.
pub fn detect_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains(SESSION_DIR)
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == "jsonl")
}

/// Fold one JSONL record into the session.
fn apply(session: &mut Session, value: &Value) {
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match kind {
        "session" => apply_header(session, value),
        "message" => apply_message(session, value),
        _ => {}
    }
}

/// The header line: the working directory and the moment the session started.
fn apply_header(session: &mut Session, value: &Value) {
    if session.cwd.is_none()
        && let Some(cwd) = string_field(value, "cwd")
    {
        session.cwd = Some(cwd.into());
    }
    if session.started_at.is_none()
        && let Some(created_ms) = value.get("created_ms").and_then(Value::as_i64)
    {
        let nanos = i128::from(created_ms) * 1_000_000;
        if let Ok(start) = OffsetDateTime::from_unix_timestamp_nanos(nanos) {
            session.started_at = Some(start);
        }
    }
}

/// A message: a role and a list of content blocks.
fn apply_message(session: &mut Session, value: &Value) {
    let Some(message) = value.get("message") else {
        return;
    };
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return;
    };

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
                match role {
                    "user" => push(session, EventKind::User { text }),
                    "assistant" => push(session, EventKind::Assistant { text }),
                    _ => push(session, EventKind::Notice { text }),
                }
            }
            "tool_call" => {
                let name = string_field(block, "name").unwrap_or_else(|| "tool".into());
                let input = block.get("input").cloned().unwrap_or(Value::Null);
                let mut event = new_event(EventKind::ToolCall { name, input });
                event.tool_call_id = string_field(block, "id");
                push_event(session, event);
            }
            "tool_result" => {
                let output = string_field(block, "output").unwrap_or_default();
                let is_error = block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let mut event = new_event(EventKind::ToolResult { output, is_error });
                event.tool_call_id = string_field(block, "id");
                push_event(session, event);
            }
            "opaque" => {
                let provider = string_field(block, "provider").unwrap_or_else(|| "unknown".into());
                push(
                    session,
                    EventKind::Notice {
                        text: format!("[opaque {provider} payload]"),
                    },
                );
            }
            _ => {}
        }
    }
}

/// Build an event.
fn new_event(kind: EventKind) -> Event {
    Event {
        index: 0,
        kind,
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    }
}

/// Append an event and stamp it with its position in the session.
fn push(session: &mut Session, kind: EventKind) {
    push_event(session, new_event(kind));
}

/// Append an event and stamp it with its position in the session.
fn push_event(session: &mut Session, mut event: Event) {
    event.index = session.events.len();
    session.events.push(event);
}
