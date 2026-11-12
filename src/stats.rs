//! Works out what a session did, and shows it.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{Value, json};
use time::OffsetDateTime;

use crate::model::{Session, SessionSource, Usage};

/// What a session did.
#[derive(Debug, Clone)]
pub struct Stats {
    pub id: String,
    pub source: SessionSource,
    pub cwd: Option<PathBuf>,
    pub model: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub duration: Option<Duration>,
    pub events: usize,
    pub tool_calls: BTreeMap<String, usize>,
    pub files_touched: Vec<String>,
    pub usage: Usage,
    /// The longest a single tool call took, and which tool it was.
    pub slowest_tool: Option<(String, Duration)>,
}

/// Work out the statistics of a session.
pub fn compute(session: &Session) -> Stats {
    let slowest_tool = session
        .tool_calls()
        .filter_map(|event| {
            event.duration_ms.map(|ms| {
                (
                    event.tool_name().unwrap_or_default().to_string(),
                    Duration::from_millis(ms),
                )
            })
        })
        .max_by_key(|(_, ms)| *ms);

    Stats {
        id: session.id.clone(),
        source: session.source,
        cwd: session.cwd.clone(),
        model: session.model.clone(),
        started_at: session.started_at,
        duration: session.duration(),
        events: session.events.len(),
        tool_calls: session.tool_counts(),
        files_touched: session.files_touched(),
        usage: session.total_usage(),
        slowest_tool,
    }
}

/// The statistics as JSON, for scripting.
pub fn to_json(stats: &Stats) -> Value {
    json!({
        "id": stats.id,
        "source": stats.source.as_str(),
        "cwd": stats.cwd.as_ref().map(|p| p.display().to_string()),
        "model": stats.model,
        "started_at": stats.started_at.map(|t| t.unix_timestamp()),
        "duration_seconds": stats.duration.map(|d| d.as_secs_f64()),
        "events": stats.events,
        "tool_calls": stats.tool_calls,
        "tool_call_total": stats.tool_calls.values().sum::<usize>(),
        "files_touched": stats.files_touched,
        "tokens": {
            "input": stats.usage.input_tokens,
            "output": stats.usage.output_tokens,
            "cache_read": stats.usage.cache_read_tokens,
            "cache_creation": stats.usage.cache_creation_tokens,
            "reasoning": stats.usage.reasoning_tokens,
            "total": stats.usage.total(),
        },
        "slowest_tool": stats.slowest_tool.as_ref().map(|(name, ms)| {
            json!({"name": name, "duration_ms": ms.as_millis() as u64})
        }),
    })
}

/// The statistics as text, for a terminal.
pub fn to_text(stats: &Stats) -> String {
    let mut out = String::new();

    let mut row = |label: &str, value: &str| {
        out.push_str(&format!("{label:<16}{value}\n"));
    };

    row("session", &stats.id);
    row("format", stats.source.as_str());
    if let Some(cwd) = &stats.cwd {
        row("working dir", &cwd.display().to_string());
    }
    if let Some(model) = &stats.model {
        row("model", model);
    }
    if let Some(start) = stats.started_at {
        row("started", &start.to_string());
    }
    match stats.duration {
        Some(duration) => row("duration", &format_duration(duration)),
        None => row("duration", "not recorded"),
    }
    row("events", &stats.events.to_string());

    out.push('\n');
    out.push_str("tool calls\n");
    if stats.tool_calls.is_empty() {
        out.push_str("  none\n");
    } else {
        let total: usize = stats.tool_calls.values().sum();
        let widest = stats
            .tool_calls
            .keys()
            .map(|name| name.len())
            .max()
            .unwrap_or(0);
        for (name, count) in &stats.tool_calls {
            out.push_str(&format!("  {name:<widest$}  {count} of {total}\n"));
        }
    }
    if let Some((name, duration)) = &stats.slowest_tool {
        out.push_str(&format!(
            "  slowest: {name} at {}\n",
            format_duration(*duration)
        ));
    }

    out.push('\n');
    out.push_str("files touched\n");
    if stats.files_touched.is_empty() {
        out.push_str("  none\n");
    } else {
        for path in &stats.files_touched {
            out.push_str(&format!("  {path}\n"));
        }
    }

    out.push('\n');
    out.push_str("tokens\n");
    let mut token = |label: &str, value: u64| {
        out.push_str(&format!("  {label:<14}{value}\n"));
    };
    token("input", stats.usage.input_tokens);
    token("output", stats.usage.output_tokens);
    token("cache read", stats.usage.cache_read_tokens);
    token("cache create", stats.usage.cache_creation_tokens);
    token("reasoning", stats.usage.reasoning_tokens);
    token("total", stats.usage.total());

    out
}

/// Format a duration as a short, readable string.
fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!(
            "{}h {}m {}s",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        )
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}.{}s", seconds, duration.subsec_millis() / 100)
    }
}
