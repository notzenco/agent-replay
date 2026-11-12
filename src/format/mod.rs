//! Reading agent session logs into the normalised model.
//!
//! Each format lives in its own module and implements [`SessionReader`]. The
//! format of a file is worked out from its contents, because the same agent
//! writes to different places depending on how it was started.

pub mod axio;
pub mod claude;
pub mod codex;

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::model::Session;

pub use crate::model::SessionSource;

/// Reads one session log format into the normalised model.
pub trait SessionReader {
    /// Parse a whole session file.
    fn read(path: &Path) -> Result<Session>;

    /// Whether this reader recognises the file. Content-based, so it can be
    /// used on files that are not in the agent's usual directory.
    fn detect(path: &Path) -> bool;
}

/// How many lines of a file are inspected when working out its format.
const DETECT_LINES: usize = 40;

/// Work out which format a session file is in.
///
/// The contents decide it. The path is only consulted for files whose contents
/// are inconclusive, which happens for truncated logs.
pub fn detect(path: &Path) -> Result<SessionSource> {
    let head = read_head(path, DETECT_LINES)?;

    if head
        .iter()
        .any(|v| v.get("payload").is_some() && is_codex_type(v))
    {
        return Ok(SessionSource::Codex);
    }
    if head.iter().any(|v| {
        v.get("type").and_then(Value::as_str) == Some("session") && v.get("version").is_some()
    }) {
        return Ok(SessionSource::Axio);
    }
    if head.iter().any(|v| {
        v.get("message").is_some()
            && matches!(
                v.get("type").and_then(Value::as_str),
                Some("user" | "assistant" | "system")
            )
    }) {
        return Ok(SessionSource::ClaudeCode);
    }

    if claude::detect_path(path) {
        return Ok(SessionSource::ClaudeCode);
    }
    if codex::detect_path(path) {
        return Ok(SessionSource::Codex);
    }
    if axio::detect_path(path) {
        return Ok(SessionSource::Axio);
    }

    bail!(
        "could not tell which agent wrote {}: it matches none of the supported formats \
         (claude code jsonl, codex rollout jsonl, axios session jsonl)",
        path.display()
    );
}

/// Read a session file with whichever format it is in.
pub fn read(path: &Path) -> Result<Session> {
    match detect(path)? {
        SessionSource::ClaudeCode => <claude::ClaudeCode as SessionReader>::read(path),
        SessionSource::Codex => <codex::Codex as SessionReader>::read(path),
        SessionSource::Axio => <axio::Axio as SessionReader>::read(path),
    }
}

/// The first `count` JSON objects of a file, skipping unparsable lines.
pub(crate) fn read_head(path: &Path, count: usize) -> Result<Vec<Value>> {
    let file =
        File::open(path).with_context(|| format!("opening session log {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line.with_context(|| format!("reading {}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            out.push(value);
            if out.len() >= count {
                break;
            }
        }
    }
    Ok(out)
}

/// Whether a JSON object looks like a Codex rollout record.
fn is_codex_type(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("session_meta" | "response_item" | "event_msg" | "turn_context")
    )
}

/// Parse an RFC 3339 timestamp, as every supported format writes them.
pub(crate) fn parse_timestamp(text: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(text, &Rfc3339).ok()
}

/// Read a string field out of a JSON object.
pub(crate) fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Read an unsigned integer field out of a JSON object.
pub(crate) fn u64_field(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(Value::as_u64)
}
