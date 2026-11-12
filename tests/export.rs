//! Exporting a session as a self-contained HTML file.

use std::path::{Path, PathBuf};

use agent_replay::export::{self, ExportOptions};
use agent_replay::format;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn html_of(name: &str, options: ExportOptions) -> String {
    let session = format::read(&fixture(name)).unwrap();
    export::to_html(&session, &options)
}

#[test]
fn produces_a_complete_html_document() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<html lang=\"en\">"));
    assert!(html.trim_end().ends_with("</html>"));
}

#[test]
fn makes_no_external_requests() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    assert!(
        !html.contains("http://"),
        "the file must not reference http"
    );
    assert!(
        !html.contains("https://"),
        "the file must not reference https"
    );
    assert!(!html.contains("<script"), "the file must not run scripts");
    assert!(
        !html.contains("<link"),
        "the file must not load stylesheets"
    );
    assert!(!html.contains("@import"), "the file must not import styles");
    assert!(!html.contains(" src="), "the file must not load anything");
    assert!(!html.contains("url("), "the file must not reference urls");
}

#[test]
fn every_css_variable_reference_is_well_formed() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    // A reference is written `var(--name)`. Anything else, such as the typo
    // `varresult`, silently drops the whole declaration.
    let bytes = html.as_bytes();
    let mut malformed = Vec::new();
    for (index, _) in html.match_indices("var") {
        let before_is_word = index > 0 && {
            let byte = bytes[index - 1];
            byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
        };
        if before_is_word {
            continue;
        }
        let rest = &html[index + 3..];
        if rest.starts_with('(') {
            if !rest.starts_with("(--") {
                malformed.push(index);
            }
        } else if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
            malformed.push(index);
        }
    }
    assert!(
        malformed.is_empty(),
        "the stylesheet has {} malformed variable reference(s)",
        malformed.len()
    );
}

#[test]
fn shows_the_session_details() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    assert!(html.contains("claude-fixture-0001"));
    assert!(html.contains("claude-code"));
    assert!(html.contains("/work/demo-app"));
    assert!(html.contains("claude-sonnet-4-5"));
}

#[test]
fn shows_every_event() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    assert!(html.contains("add a health endpoint"));
    assert!(html.contains("I&#39;ll add a health endpoint to the router."));
    assert!(html.contains("The test command failed"));
    assert!(html.contains("turn took 9.1s"));
}

#[test]
fn shows_a_diff_for_an_edit() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    assert!(html.contains("class=\"diff\""));
    assert!(html.contains("tr class=\"added\""));
    assert!(html.contains("tr class=\"removed\""));
    assert!(html.contains("pub fn routes() -&gt; Router"));
    assert!(html.contains("<td class=\"sign\">+</td>"));
    assert!(html.contains("<td class=\"sign\">-</td>"));
}

#[test]
fn escapes_content_that_looks_like_markup() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let mut session = session;
    session.events.push(agent_replay::model::Event {
        index: 0,
        kind: agent_replay::model::EventKind::User {
            text: "<script>alert('x')</script>".into(),
        },
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    });
    let html = export::to_html(&session, &ExportOptions::default());
    assert!(!html.contains("<script>alert"));
    assert!(html.contains("&lt;script&gt;alert"));
}

#[test]
fn redacts_by_default() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let mut session = session;
    session.events.push(agent_replay::model::Event {
        index: 0,
        kind: agent_replay::model::EventKind::User {
            text: "key AKIAIOSFODNN7EXAMPLE mail someone@example.com".into(),
        },
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    });
    let html = export::to_html(&session, &ExportOptions::default());
    assert!(html.contains("[redacted:aws-key-id]"));
    assert!(html.contains("[email]"));
    assert!(!html.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(!html.contains("someone@example.com"));
}

#[test]
fn keeps_content_when_redaction_is_turned_off() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let mut session = session;
    session.events.push(agent_replay::model::Event {
        index: 0,
        kind: agent_replay::model::EventKind::User {
            text: "key AKIAIOSFODNN7EXAMPLE mail someone@example.com".into(),
        },
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    });
    let options = ExportOptions {
        redact: false,
        highlight_honeytokens: true,
    };
    let html = export::to_html(&session, &options);
    assert!(html.contains("AKIAIOSFODNN7EXAMPLE"));
    assert!(html.contains("someone@example.com"));
}

#[test]
fn highlights_a_honeytoken_and_names_it() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let mut session = session;
    let id = agent_replay::honeytoken::id_with_check_chars("ABCDEF");
    session.events.push(agent_replay::model::Event {
        index: 0,
        kind: agent_replay::model::EventKind::User {
            text: format!("the value leaked TRIPWIRE{id} here"),
        },
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    });
    let html = export::to_html(&session, &ExportOptions::default());
    assert!(html.contains("<mark class=\"honeytoken\">"));
    assert!(html.contains("TRIPWIRE"));
    assert!(html.contains("honeytoken(s) found"));
    assert!(html.contains(&id));
}

#[test]
fn redacts_the_header_file_list_and_diff_paths() {
    let home = std::env::var("HOME").unwrap();
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let mut session = session;
    session.cwd = Some(PathBuf::from(format!("{home}/demo")));
    session.events.push(agent_replay::model::Event {
        index: 0,
        kind: agent_replay::model::EventKind::ToolCall {
            name: "Write".into(),
            input: serde_json::json!({
                "file_path": format!("{home}/demo/notes.txt"),
                "content": "hello\n"
            }),
        },
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    });
    let html = export::to_html(&session, &ExportOptions::default());
    assert!(
        !html.contains(&home),
        "the home directory must not appear anywhere in the export"
    );
    assert!(html.contains("~/demo"));
    assert!(html.contains("~/demo/notes.txt"));
}

#[test]
fn names_honeytokens_above_the_timeline() {
    let session = format::read(&fixture("claude-session.jsonl")).unwrap();
    let mut session = session;
    let id = agent_replay::honeytoken::id_with_check_chars("ABCDEF");
    session.events.push(agent_replay::model::Event {
        index: 0,
        kind: agent_replay::model::EventKind::User {
            text: format!("the value leaked TRIPWIRE{id} here"),
        },
        ts: None,
        duration_ms: None,
        usage: None,
        model: None,
        tool_call_id: None,
    });
    let html = export::to_html(&session, &ExportOptions::default());
    let banner = html.find("class=\"honeytokens\"").unwrap();
    let timeline = html.find("class=\"timeline\"").unwrap();
    assert!(
        banner < timeline,
        "the honeytoken banner belongs at the top"
    );
}

#[test]
fn is_deterministic() {
    let first = html_of("claude-session.jsonl", ExportOptions::default());
    let second = html_of("claude-session.jsonl", ExportOptions::default());
    assert_eq!(first, second);
}

#[test]
fn summarises_the_session() {
    let html = html_of("claude-session.jsonl", ExportOptions::default());
    assert!(html.contains("Tool calls"));
    assert!(html.contains("Files touched"));
    assert!(html.contains("/work/demo-app/src/routes.rs"));
}
