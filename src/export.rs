//! Renders a session as one self-contained HTML file.
//!
//! The file makes no external requests: the stylesheet is inline, there is no
//! JavaScript, and no image or font is loaded from anywhere else. It works
//! from a `file://` URL, offline, and can be attached to a bug report.

use std::fmt::Write as _;
use std::time::Duration;

use serde_json::Value;
use time::OffsetDateTime;
use time::macros::format_description;

use crate::diff;
use crate::model::{Event, EventKind, Session};
use crate::redact::Redactor;

/// How an export should be produced.
#[derive(Debug, Clone, Copy)]
pub struct ExportOptions {
    /// Whether secrets, home paths and emails are replaced.
    pub redact: bool,
    /// Whether honeytokens are found and highlighted.
    pub highlight_honeytokens: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            redact: true,
            highlight_honeytokens: true,
        }
    }
}

/// The time format used for the header.
const HEADER_TIME: &[time::format_description::FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

/// The time format used for each event.
const EVENT_TIME: &[time::format_description::FormatItem<'_>] =
    format_description!("[hour]:[minute]:[second]");

/// Render a session as a self-contained HTML document.
pub fn to_html(session: &Session, options: &ExportOptions) -> String {
    let redactor =
        Redactor::new(options.redact).highlight_honeytokens(options.highlight_honeytokens);

    let mut out = String::new();
    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    out.push_str(
        "<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n",
    );
    let _ = writeln!(
        out,
        "<title>agent-replay · {}</title>",
        escape_html(&session.id)
    );
    out.push_str(STYLE);
    out.push_str("</head>\n<body>\n");

    // The sections are rendered in timeline order but assembled so the
    // honeytoken banner, which is only known once everything has been
    // scanned, still lands at the top of the file.
    let mut honeytokens: Vec<String> = Vec::new();
    let mut header = String::new();
    render_header(&mut header, session, &redactor, &mut honeytokens);
    let mut summary = String::new();
    render_summary(&mut summary, session, &redactor, &mut honeytokens);
    let mut timeline = String::new();
    render_timeline(&mut timeline, session, &redactor, &mut honeytokens);

    out.push_str(&header);
    render_honeytokens(&mut out, &honeytokens);
    out.push_str(&summary);
    out.push_str(&timeline);

    out.push_str("<footer class=\"footer\">");
    out.push_str(
        "Rendered by agent-replay. Redaction is applied by default; secrets, \
         home paths and email addresses have been replaced.",
    );
    out.push_str("</footer>\n</body>\n</html>\n");
    out
}

/// The banner naming the honeytokens the session leaked.
fn render_honeytokens(out: &mut String, honeytokens: &[String]) {
    if honeytokens.is_empty() {
        return;
    }
    out.push_str("<aside class=\"honeytokens\">\n");
    let _ = writeln!(
        out,
        "<strong>{} agent-tripwire honeytoken(s) found.</strong>",
        honeytokens.len()
    );
    out.push_str(
        "<p>These are planted credentials that should never have appeared in a transcript. ",
    );
    out.push_str("The values are shown so the leak can be traced.</p>\n<ul>\n");
    for id in honeytokens {
        let _ = writeln!(out, "<li><code>{}</code></li>", escape_html(id));
    }
    out.push_str("</ul>\n</aside>\n");
}

/// The session header: what it was, where it ran and when.
fn render_header(
    out: &mut String,
    session: &Session,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    out.push_str("<header class=\"header\">\n");
    let _ = writeln!(
        out,
        "<h1>{} <span class=\"source\">{}</span></h1>",
        escape_html(&session.id),
        escape_html(session.source.as_str())
    );
    out.push_str("<dl class=\"meta\">\n");

    let mut meta = |label: &str, value: String| {
        let _ = writeln!(
            out,
            "<div><dt>{}</dt><dd>{}</dd></div>",
            escape_html(label),
            value
        );
    };
    meta(
        "Working directory",
        redacted_html(
            &session
                .cwd
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not recorded".to_string()),
            redactor,
            honeytokens,
        ),
    );
    meta(
        "Model",
        redacted_html(
            session.model.as_deref().unwrap_or("not recorded"),
            redactor,
            honeytokens,
        ),
    );
    if let Some(start) = session.started_at {
        meta("Started", escape_html(&format_time(start, HEADER_TIME)));
    }
    if let Some(duration) = session.duration() {
        meta("Duration", escape_html(&format_duration(duration)));
    }
    meta("Events", session.events.len().to_string());
    out.push_str("</dl>\n</header>\n");
}

/// The summary bar: how much the session did.
fn render_summary(
    out: &mut String,
    session: &Session,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    let usage = session.total_usage();
    let counts = session.tool_counts();
    let files = session.files_touched();

    out.push_str("<section class=\"summary\">\n");
    let mut tile = |label: &str, value: String| {
        let _ = writeln!(
            out,
            "<div class=\"tile\"><span class=\"value\">{}</span><span class=\"label\">{}</span></div>",
            escape_html(&value),
            escape_html(label)
        );
    };
    tile("tool calls", session.tool_calls().count().to_string());
    tile("files touched", files.len().to_string());
    tile("input tokens", usage.total_input().to_string());
    tile("output tokens", usage.total_output().to_string());
    tile("reasoning tokens", usage.reasoning_tokens.to_string());
    out.push_str("</section>\n");

    if !counts.is_empty() {
        out.push_str("<section class=\"counts\">\n<h2>Tool calls</h2>\n<ul>\n");
        for (name, count) in counts {
            let _ = writeln!(
                out,
                "<li><span class=\"tool\">{}</span><span class=\"count\">{}</span></li>",
                escape_html(&name),
                count
            );
        }
        out.push_str("</ul>\n</section>\n");
    }

    if !files.is_empty() {
        out.push_str("<section class=\"files\">\n<h2>Files touched</h2>\n<ul>\n");
        for path in files {
            out.push_str("<li><code>");
            write_redacted(out, &path, redactor, honeytokens);
            out.push_str("</code></li>\n");
        }
        out.push_str("</ul>\n</section>\n");
    }
}

/// The timeline: one card per event.
fn render_timeline(
    out: &mut String,
    session: &Session,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    out.push_str("<section class=\"timeline\">\n<h2>Timeline</h2>\n");
    let start = session.started_at;

    for event in &session.events {
        let mut classes = event.kind.name().to_string();
        if matches!(&event.kind, EventKind::ToolResult { is_error: true, .. }) {
            classes.push_str(" error");
        }
        let _ = writeln!(out, "<article class=\"event {classes}\">");
        out.push_str("<header class=\"event-head\">");
        out.push_str("<span class=\"time\">");
        match (start, event.ts) {
            (Some(start), Some(ts)) => {
                let offset = (ts - start).unsigned_abs();
                let _ = write!(
                    out,
                    "{} <span class=\"offset\">+{}</span>",
                    escape_html(&format_time(ts, EVENT_TIME)),
                    escape_html(&format_offset(offset))
                );
            }
            (None, Some(ts)) => {
                let _ = write!(out, "{}", escape_html(&format_time(ts, EVENT_TIME)));
            }
            _ => {
                out.push('—');
            }
        }
        out.push_str("</span> ");
        let _ = write!(
            out,
            "<span class=\"badge\">{}</span>",
            escape_html(event.kind.name())
        );
        if let Some(tool) = event.tool_name() {
            let _ = write!(
                out,
                " <span class=\"tool-name\">{}</span>",
                escape_html(tool)
            );
        }
        out.push_str("</header>\n");

        render_body(out, event, redactor, honeytokens);

        if let Some(usage) = event.usage {
            let _ = writeln!(
                out,
                "<footer class=\"usage\">in {} · out {} · cache read {}</footer>",
                usage.input_tokens, usage.output_tokens, usage.cache_read_tokens
            );
        }
        out.push_str("</article>\n");
    }

    out.push_str("</section>\n");
}

/// The body of one event.
fn render_body(
    out: &mut String,
    event: &Event,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    match &event.kind {
        EventKind::User { text } | EventKind::Assistant { text } => {
            out.push_str("<div class=\"prose\">");
            write_redacted(out, text, redactor, honeytokens);
            out.push_str("</div>\n");
        }
        EventKind::Reasoning { text } => {
            out.push_str(
                "<details class=\"reasoning\"><summary>Reasoning</summary><div class=\"prose\">",
            );
            write_redacted(out, text, redactor, honeytokens);
            out.push_str("</div></details>\n");
        }
        EventKind::Notice { text } => {
            out.push_str("<div class=\"notice\">");
            write_redacted(out, text, redactor, honeytokens);
            out.push_str("</div>\n");
        }
        EventKind::ToolCall { name, input } => {
            out.push_str("<div class=\"tool-call\">\n");
            render_input(out, input, redactor, honeytokens);
            out.push_str("</div>\n");
            render_diff(out, name, input, redactor, honeytokens);
        }
        EventKind::ToolResult { output, is_error } => {
            let class = if *is_error { "output error" } else { "output" };
            let _ = write!(out, "<pre class=\"{class}\">");
            write_redacted(out, output, redactor, honeytokens);
            out.push_str("</pre>\n");
        }
    }
}

/// The input of a tool call, as a list of fields.
fn render_input(
    out: &mut String,
    input: &Value,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    match input {
        Value::Object(fields) if !fields.is_empty() => {
            out.push_str("<dl class=\"fields\">\n");
            for (key, value) in fields {
                let _ = write!(out, "<div><dt>{}</dt><dd>", escape_html(key));
                write_value(out, value, redactor, honeytokens);
                out.push_str("</dd></div>\n");
            }
            out.push_str("</dl>\n");
        }
        Value::Null => {}
        other => {
            out.push_str("<div class=\"value\">");
            write_value(out, other, redactor, honeytokens);
            out.push_str("</div>\n");
        }
    }
}

/// The diff of a file-editing tool call.
fn render_diff(
    out: &mut String,
    name: &str,
    input: &Value,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    let Some(diff) = diff::diff_for(name, input) else {
        return;
    };
    out.push_str("<div class=\"diff-head\">");
    write_redacted(
        out,
        diff.path.as_deref().unwrap_or("file"),
        redactor,
        honeytokens,
    );
    let _ = writeln!(
        out,
        " <span class=\"added\">+{}</span> <span class=\"removed\">−{}</span></div>",
        diff.added, diff.removed
    );
    out.push_str("<table class=\"diff\">\n");
    for line in &diff.lines {
        let class = match line.kind {
            diff::DiffLineKind::Added => "added",
            diff::DiffLineKind::Removed => "removed",
            diff::DiffLineKind::Context => "context",
        };
        let _ = write!(out, "<tr class=\"{class}\">");
        let _ = write!(out, "<td class=\"sign\">{}</td>", line.kind.prefix());
        out.push_str("<td class=\"code\">");
        write_redacted(out, &line.text, redactor, honeytokens);
        out.push_str("</td></tr>\n");
    }
    out.push_str("</table>\n");
    if diff.is_truncated() {
        let _ = writeln!(
            out,
            "<p class=\"truncated\">{} of {} lines shown</p>",
            diff.lines.len(),
            diff.total_lines
        );
    }
}

/// A value from a tool's input.
fn write_value(
    out: &mut String,
    value: &Value,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    match value {
        Value::String(text) => {
            out.push_str("<span class=\"text\">");
            write_redacted(out, text, redactor, honeytokens);
            out.push_str("</span>");
        }
        Value::Null => out.push_str("<span class=\"null\">null</span>"),
        Value::Bool(flag) => {
            let _ = write!(out, "<span class=\"bool\">{}</span>", flag);
        }
        Value::Number(number) => {
            let _ = write!(out, "<span class=\"number\">{}</span>", number);
        }
        other => {
            let text = serde_json::to_string_pretty(other).unwrap_or_default();
            out.push_str("<pre class=\"json\">");
            write_redacted(out, &text, redactor, honeytokens);
            out.push_str("</pre>");
        }
    }
}

/// Redacted, escaped HTML for one short value, such as a header field.
fn redacted_html(text: &str, redactor: &Redactor, honeytokens: &mut Vec<String>) -> String {
    let mut out = String::new();
    write_redacted(&mut out, text, redactor, honeytokens);
    out
}

/// Write text that has been redacted, marking any honeytoken it holds.
fn write_redacted(
    out: &mut String,
    text: &str,
    redactor: &Redactor,
    honeytokens: &mut Vec<String>,
) {
    let redaction = redactor.redact(text);
    honeytokens.extend(redaction.honeytokens.iter().cloned());
    for segment in &redaction.segments {
        let escaped = escape_html(&segment.text);
        if segment.honeytoken {
            out.push_str("<mark class=\"honeytoken\">");
            out.push_str(&escaped);
            out.push_str("</mark>");
        } else {
            out.push_str(&escaped);
        }
    }
}

/// Escape text so it cannot break out of the HTML it is put into.
pub fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push_str(c.encode_utf8(&mut [0u8; 4])),
        }
    }
    out
}

/// Format a duration as a short, readable string.
pub fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 3600 {
        format!("{}h {}m", seconds / 3600, (seconds % 3600) / 60)
    } else if seconds >= 60 {
        format!("{}m {}s", seconds / 60, seconds % 60)
    } else {
        format!("{}.{}s", seconds, duration.subsec_millis() / 100)
    }
}

/// Format a duration as an offset from the start of the session.
fn format_offset(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Format a timestamp.
fn format_time(
    time: OffsetDateTime,
    format: &[time::format_description::FormatItem<'_>],
) -> String {
    time.format(format).unwrap_or_default()
}

/// The stylesheet, inline so the file needs nothing from anywhere else.
const STYLE: &str = r#"<style>
:root {
  color-scheme: light dark;
  --bg: #ffffff;
  --fg: #1b1f24;
  --muted: #5b6570;
  --line: #d8dee4;
  --panel: #f6f8fa;
  --user: #0969da;
  --assistant: #8250df;
  --tool: #bf8700;
  --result: #1a7f37;
  --error: #cf222e;
  --notice: #6e7781;
  --added-bg: #dafbe1;
  --added-fg: #0a5527;
  --removed-bg: #ffebe9;
  --removed-fg: #82071e;
  --honey: #fff3c4;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #0d1117;
    --fg: #e6edf3;
    --muted: #9198a1;
    --line: #30363d;
    --panel: #161b22;
    --user: #58a6ff;
    --assistant: #bc8cff;
    --tool: #d29922;
    --result: #3fb950;
    --error: #ff7b72;
    --notice: #8b949e;
    --added-bg: #12261e;
    --added-fg: #7ee2a8;
    --removed-bg: #25171c;
    --removed-fg: #ffa198;
    --honey: #4a3c00;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0;
  padding: 2rem 1rem 4rem;
  background: var(--bg);
  color: var(--fg);
  font: 15px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
}
header, section, footer { max-width: 62rem; margin: 0 auto; }
h1 { font-size: 1.4rem; margin: 0 0 .75rem; display: flex; align-items: baseline; gap: .6rem; }
h2 { font-size: 1rem; margin: 1.75rem 0 .6rem; color: var(--muted); text-transform: uppercase; letter-spacing: .06em; }
.source { font-size: .8rem; font-weight: 500; color: var(--muted); border: 1px solid var(--line); border-radius: 999px; padding: .1rem .55rem; }
.meta { display: flex; flex-wrap: wrap; gap: .4rem 2rem; margin: 0; padding: .85rem 1rem; background: var(--panel); border: 1px solid var(--line); border-radius: .5rem; }
.meta div { display: flex; gap: .5rem; align-items: baseline; }
.meta dt { color: var(--muted); font-size: .8rem; text-transform: uppercase; letter-spacing: .04em; }
.meta dd { margin: 0; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .85rem; }
.summary { display: flex; flex-wrap: wrap; gap: .6rem; margin-top: 1rem; }
.tile { flex: 1 1 8rem; display: flex; flex-direction: column; gap: .15rem; padding: .7rem .9rem; background: var(--panel); border: 1px solid var(--line); border-radius: .5rem; }
.tile .value { font-size: 1.3rem; font-weight: 600; font-variant-numeric: tabular-nums; }
.tile .label { font-size: .75rem; color: var(--muted); text-transform: uppercase; letter-spacing: .05em; }
.counts ul, .files ul { list-style: none; margin: 0; padding: 0; display: flex; flex-wrap: wrap; gap: .4rem; }
.counts li { display: flex; align-items: baseline; gap: .45rem; padding: .2rem .6rem; border: 1px solid var(--line); border-radius: .35rem; background: var(--panel); }
.counts .tool { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .85rem; }
.counts .count { color: var(--muted); font-size: .8rem; }
.files code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .82rem; }
.event { border: 1px solid var(--line); border-left-width: 3px; border-radius: .5rem; padding: .7rem .9rem; margin-bottom: .6rem; background: var(--bg); }
.event.user { border-left-color: var(--user); }
.event.assistant { border-left-color: var(--assistant); }
.event.tool { border-left-color: var(--tool); }
.event.result { border-left-color: var(--result); }
.event.result.error { border-left-color: var(--error); }
.event.reasoning { border-left-color: var(--notice); opacity: .85; }
.event.notice { border-left-color: var(--notice); }
.event-head { display: flex; align-items: baseline; gap: .55rem; font-size: .8rem; color: var(--muted); margin-bottom: .45rem; }
.time { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
.offset { opacity: .7; }
.badge { text-transform: uppercase; letter-spacing: .06em; font-weight: 600; font-size: .68rem; }
.event.user .badge { color: var(--user); }
.event.assistant .badge { color: var(--assistant); }
.event.tool .badge { color: var(--tool); }
.event.result .badge { color: var(--result); }
.event.result.error .badge { color: var(--error); }
.tool-name { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-weight: 600; color: var(--fg); }
.prose { white-space: pre-wrap; word-break: break-word; }
.notice { color: var(--muted); font-style: italic; }
.reasoning summary { cursor: pointer; color: var(--muted); font-size: .8rem; }
.fields { margin: 0; display: grid; gap: .3rem; }
.fields div { display: grid; grid-template-columns: minmax(6rem, 10rem) 1fr; gap: .75rem; align-items: baseline; }
.fields dt { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .82rem; color: var(--muted); }
.fields dd { margin: 0; min-width: 0; word-break: break-word; }
pre.output, pre.json { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .82rem; background: var(--panel); border: 1px solid var(--line); border-radius: .4rem; padding: .55rem .7rem; margin: .5rem 0 0; overflow-x: auto; white-space: pre-wrap; word-break: break-word; }
pre.output.error { border-color: var(--error); color: var(--error); }
.diff-head { margin-top: .6rem; font-size: .8rem; color: var(--muted); font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
.diff-head .added { color: var(--added-fg); }
.diff-head .removed { color: var(--removed-fg); }
table.diff { width: 100%; border-collapse: collapse; margin-top: .35rem; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .8rem; }
table.diff td { padding: 0 .4rem; vertical-align: top; white-space: pre-wrap; word-break: break-word; }
table.diff td.sign { width: 1ch; text-align: center; user-select: none; }
table.diff tr.added { background: var(--added-bg); color: var(--added-fg); }
table.diff tr.removed { background: var(--removed-bg); color: var(--removed-fg); }
table.diff tr.context { color: var(--muted); }
.truncated { font-size: .78rem; color: var(--muted); margin: .35rem 0 0; }
.usage { margin-top: .45rem; font-size: .72rem; color: var(--muted); font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
mark.honeytoken { background: var(--honey); color: var(--fg); font-weight: 600; padding: 0 .15rem; border-radius: .2rem; }
.honeytokens { margin-top: 1.5rem; padding: .9rem 1.1rem; border: 1px solid var(--tool); border-radius: .5rem; background: var(--panel); }
.honeytokens p { margin: .4rem 0; color: var(--muted); }
.honeytokens ul { margin: .3rem 0 0; padding-left: 1.2rem; }
.footer { margin-top: 2.5rem; padding-top: 1rem; border-top: 1px solid var(--line); color: var(--muted); font-size: .8rem; }
@media (max-width: 40rem) {
  body { padding: 1rem .6rem 3rem; }
  .fields div { grid-template-columns: 1fr; gap: .15rem; }
}
</style>
"#;
