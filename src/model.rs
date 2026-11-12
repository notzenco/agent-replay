//! The normalised event model every session format is read into.
//!
//! Formats differ in how they spell a tool call, when they stamp a record and
//! whether they report token usage. Nothing downstream of this module — the
//! TUI, the stats command, the HTML export — is allowed to know which agent
//! produced a log.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use time::OffsetDateTime;

/// The agent whose log a session came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SessionSource {
    ClaudeCode,
    Codex,
    Axio,
}

impl SessionSource {
    /// Short machine-readable name, used in the CLI and in exports.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionSource::ClaudeCode => "claude-code",
            SessionSource::Codex => "codex",
            SessionSource::Axio => "axios",
        }
    }

    /// Every source, in the order `agent-replay list` reports them.
    pub fn all() -> [SessionSource; 3] {
        [
            SessionSource::ClaudeCode,
            SessionSource::Codex,
            SessionSource::Axio,
        ]
    }
}

/// Token counts for one assistant turn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Usage {
    /// Tokens sent to the model, counting cache reads.
    pub fn total_input(&self) -> u64 {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }

    /// Tokens produced by the model.
    pub fn total_output(&self) -> u64 {
        self.output_tokens
    }

    /// Every token, in or out.
    pub fn total(&self) -> u64 {
        self.total_input() + self.total_output()
    }

    /// The difference between two readings of a cumulative counter.
    pub fn difference(self, previous: Usage) -> Usage {
        Usage {
            input_tokens: self.input_tokens.saturating_sub(previous.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(previous.output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_sub(previous.cache_read_tokens),
            cache_creation_tokens: self
                .cache_creation_tokens
                .saturating_sub(previous.cache_creation_tokens),
            reasoning_tokens: self
                .reasoning_tokens
                .saturating_sub(previous.reasoning_tokens),
        }
    }
}

/// What a single entry in the timeline is.
#[derive(Debug, Clone, PartialEq)]
pub enum EventKind {
    /// A prompt from the user.
    User { text: String },
    /// Prose from the assistant.
    Assistant { text: String },
    /// The assistant's private reasoning, when the format records it.
    Reasoning { text: String },
    /// A tool the assistant asked for.
    ToolCall { name: String, input: Value },
    /// The output of a tool the assistant asked for.
    ToolResult { output: String, is_error: bool },
    /// Anything else worth showing: system notices, hooks, plan changes.
    Notice { text: String },
}

impl EventKind {
    /// Stable lowercase name, used for filtering and in exports.
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::User { .. } => "user",
            EventKind::Assistant { .. } => "assistant",
            EventKind::Reasoning { .. } => "reasoning",
            EventKind::ToolCall { .. } => "tool",
            EventKind::ToolResult { .. } => "result",
            EventKind::Notice { .. } => "notice",
        }
    }

    /// Two-letter tag for the timeline.
    pub fn tag(&self) -> &'static str {
        match self {
            EventKind::User { .. } => "us",
            EventKind::Assistant { .. } => "as",
            EventKind::Reasoning { .. } => "th",
            EventKind::ToolCall { .. } => "tl",
            EventKind::ToolResult { .. } => "rs",
            EventKind::Notice { .. } => "nt",
        }
    }

    /// The text an event carries, if it carries any.
    pub fn text(&self) -> Option<&str> {
        match self {
            EventKind::User { text }
            | EventKind::Assistant { text }
            | EventKind::Reasoning { text }
            | EventKind::Notice { text } => Some(text),
            EventKind::ToolCall { .. } => None,
            EventKind::ToolResult { output, .. } => Some(output),
        }
    }

    /// One-line summary for the timeline.
    pub fn summary(&self) -> String {
        match self {
            EventKind::User { text } | EventKind::Assistant { text } => first_line(text),
            EventKind::Reasoning { text } => first_line(text),
            EventKind::Notice { text } => first_line(text),
            EventKind::ToolCall { name, .. } => name.clone(),
            EventKind::ToolResult { is_error, .. } => {
                if *is_error {
                    "error".to_string()
                } else {
                    "ok".to_string()
                }
            }
        }
    }
}

/// One entry in the timeline.
#[derive(Debug, Clone)]
pub struct Event {
    /// Position in the session, starting at zero.
    pub index: usize,
    pub kind: EventKind,
    /// When the event happened. Not every format stamps every record.
    pub ts: Option<OffsetDateTime>,
    /// How long the tool took, where the format reports it.
    pub duration_ms: Option<u64>,
    /// Token usage, for assistant turns.
    pub usage: Option<Usage>,
    /// The model that produced the event, where the format reports it.
    pub model: Option<String>,
    /// Links a tool call to its result.
    pub tool_call_id: Option<String>,
}

impl Event {
    /// The tool name, if this event is a tool call.
    pub fn tool_name(&self) -> Option<&str> {
        match &self.kind {
            EventKind::ToolCall { name, .. } => Some(name),
            _ => None,
        }
    }

    /// The file or command a tool call acted on, if it has one.
    pub fn target(&self) -> Option<String> {
        let EventKind::ToolCall { input, .. } = &self.kind else {
            return None;
        };
        tool_target(input)
    }
}

/// A whole session, normalised.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub source: SessionSource,
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub events: Vec<Event>,
}

impl Session {
    /// Wall-clock time between the first and last stamped event.
    pub fn duration(&self) -> Option<Duration> {
        let first = self.events.iter().find_map(|e| e.ts)?;
        let last = self.events.iter().rev().find_map(|e| e.ts)?;
        Some((last - first).unsigned_abs())
    }

    /// Every tool call in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = &Event> {
        self.events.iter().filter(|e| e.tool_name().is_some())
    }

    /// How many times each tool was called, in call order.
    pub fn tool_counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for event in self.tool_calls() {
            *counts
                .entry(event.tool_name().unwrap_or_default().to_string())
                .or_insert(0) += 1;
        }
        counts
    }

    /// The files the session wrote or edited, in first-touch order.
    pub fn files_touched(&self) -> Vec<String> {
        let mut seen = Vec::new();
        for event in self.tool_calls() {
            let Some(target) = event.target() else {
                continue;
            };
            if !is_file_edit(event.tool_name().unwrap_or_default()) {
                continue;
            }
            if !seen.contains(&target) {
                seen.push(target);
            }
        }
        seen
    }

    /// Token usage summed over every assistant turn.
    pub fn total_usage(&self) -> Usage {
        let mut total = Usage::default();
        for usage in self.events.iter().filter_map(|e| e.usage) {
            total.input_tokens += usage.input_tokens;
            total.output_tokens += usage.output_tokens;
            total.cache_read_tokens += usage.cache_read_tokens;
            total.cache_creation_tokens += usage.cache_creation_tokens;
            total.reasoning_tokens += usage.reasoning_tokens;
        }
        total
    }
}

/// Whether a tool name writes to a file.
pub fn is_file_edit(tool: &str) -> bool {
    matches!(
        tool.to_ascii_lowercase().as_str(),
        "write"
            | "edit"
            | "multiedit"
            | "apply_patch"
            | "applypatch"
            | "notebookedit"
            | "str_replace"
    )
}

/// The path or command a tool call acted on, read out of its input.
pub fn tool_target(input: &Value) -> Option<String> {
    for key in ["file_path", "filePath", "path", "notebook_path"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    for key in ["command", "cmd", "script", "shell_command"] {
        if let Some(value) = input.get(key).and_then(Value::as_str) {
            return Some(value.to_string());
        }
    }
    for key in ["patch", "input", "diff", "content"] {
        if let Some(text) = input.get(key).and_then(Value::as_str)
            && let Some(path) = crate::diff::patch_file_path(text)
        {
            return Some(path);
        }
    }
    None
}

/// The first line of a text, trimmed, for use as a one-line label.
fn first_line(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_event(name: &str, input: Value) -> Event {
        Event {
            index: 0,
            kind: EventKind::ToolCall {
                name: name.to_string(),
                input,
            },
            ts: None,
            duration_ms: None,
            usage: None,
            model: None,
            tool_call_id: None,
        }
    }

    #[test]
    fn usage_totals_include_cache_reads() {
        let usage = Usage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 100,
            cache_creation_tokens: 20,
            reasoning_tokens: 3,
        };
        assert_eq!(usage.total_input(), 130);
        assert_eq!(usage.total_output(), 5);
        assert_eq!(usage.total(), 135);
    }

    #[test]
    fn tool_target_prefers_a_path_over_a_command() {
        let input = serde_json::json!({"file_path": "/tmp/a.rs", "command": "ls"});
        assert_eq!(tool_target(&input).as_deref(), Some("/tmp/a.rs"));
    }

    #[test]
    fn tool_target_falls_back_to_a_command() {
        let input = serde_json::json!({"command": "ls -la"});
        assert_eq!(tool_target(&input).as_deref(), Some("ls -la"));
    }

    #[test]
    fn tool_target_is_none_for_an_empty_input() {
        assert_eq!(tool_target(&serde_json::json!({})), None);
    }

    #[test]
    fn files_touched_lists_edited_files_once_in_order() {
        let mut session = Session {
            id: "s".into(),
            source: SessionSource::ClaudeCode,
            cwd: None,
            model: None,
            started_at: None,
            events: Vec::new(),
        };
        session.events.push(tool_event(
            "Edit",
            serde_json::json!({"file_path": "/tmp/b.rs"}),
        ));
        session.events.push(tool_event(
            "Read",
            serde_json::json!({"file_path": "/tmp/a.rs"}),
        ));
        session.events.push(tool_event(
            "Edit",
            serde_json::json!({"file_path": "/tmp/b.rs"}),
        ));
        session.events.push(tool_event(
            "Write",
            serde_json::json!({"file_path": "/tmp/c.rs"}),
        ));
        assert_eq!(session.files_touched(), vec!["/tmp/b.rs", "/tmp/c.rs"]);
    }

    #[test]
    fn tool_counts_group_by_name() {
        let mut session = Session {
            id: "s".into(),
            source: SessionSource::ClaudeCode,
            cwd: None,
            model: None,
            started_at: None,
            events: Vec::new(),
        };
        session
            .events
            .push(tool_event("Bash", serde_json::json!({})));
        session
            .events
            .push(tool_event("Bash", serde_json::json!({})));
        session
            .events
            .push(tool_event("Read", serde_json::json!({})));
        let counts = session.tool_counts();
        assert_eq!(counts.get("Bash"), Some(&2));
        assert_eq!(counts.get("Read"), Some(&1));
    }

    #[test]
    fn event_summary_is_the_first_line() {
        let kind = EventKind::User {
            text: "first line\nsecond line".into(),
        };
        assert_eq!(kind.summary(), "first line");
    }
}
