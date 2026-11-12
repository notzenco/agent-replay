//! Turns the input of a file-editing tool into a diff.
//!
//! Diffs are derived on demand from what the agent sent, rather than being
//! stored in the model, so there is one implementation of diffing for both the
//! terminal viewer and the HTML export.

use serde_json::Value;
use similar::{ChangeTag, TextDiff};

/// The most lines a diff will show before it is cut short.
const MAX_LINES: usize = 400;

/// What a line of a diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    /// Unchanged context.
    Context,
    /// A line the agent added.
    Added,
    /// A line the agent removed.
    Removed,
}

impl DiffLineKind {
    /// The prefix a line carries when the diff is written out as text.
    pub fn prefix(self) -> char {
        match self {
            DiffLineKind::Context => ' ',
            DiffLineKind::Added => '+',
            DiffLineKind::Removed => '-',
        }
    }
}

/// One line of a diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

/// A diff of one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Diff {
    /// The file the diff is about, when the tool said.
    pub path: Option<String>,
    pub lines: Vec<DiffLine>,
    /// How many lines were added, before any truncation.
    pub added: usize,
    /// How many lines were removed, before any truncation.
    pub removed: usize,
    /// How many lines the diff had before it was cut short.
    pub total_lines: usize,
}

impl Diff {
    /// Whether the diff was cut short.
    pub fn is_truncated(&self) -> bool {
        self.total_lines > self.lines.len()
    }

    /// The diff written as text, in the usual unified form.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        if let Some(path) = &self.path {
            out.push_str(&format!("--- {path}\n+++ {path}\n"));
        }
        for line in &self.lines {
            out.push(line.kind.prefix());
            out.push_str(&line.text);
            if !line.text.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }
}

/// Build the diff for a tool call, if it was one that edited a file.
///
/// `Write` is shown as a whole file being added: the log does not record what
/// the file held before, so the old side is empty. `Edit` is shown as the
/// smallest change that turns `old_string` into `new_string`.
pub fn diff_for(tool: &str, input: &Value) -> Option<Diff> {
    let name = tool.to_ascii_lowercase();
    match name.as_str() {
        "write" | "notebookedit" => write_diff(input),
        "edit" | "multiedit" | "str_replace" => edit_diff(input),
        "apply_patch" | "applypatch" => patch_diff(input),
        _ => None,
    }
}

/// A whole file written.
fn write_diff(input: &Value) -> Option<Diff> {
    let content = input.get("content").and_then(Value::as_str)?;
    let path = file_path(input);
    let mut diff = diff_between("", content);
    diff.path = path;
    Some(diff)
}

/// A single replacement inside a file.
fn edit_diff(input: &Value) -> Option<Diff> {
    let old = input.get("old_string").and_then(Value::as_str)?;
    let new = input.get("new_string").and_then(Value::as_str)?;
    let path = file_path(input);
    let mut diff = diff_between(old, new);
    diff.path = path;
    Some(diff)
}

/// The first file a Codex-style patch touches, named by its
/// `*** Update File:` / `*** Add File:` / `*** Delete File:` header.
pub fn patch_file_path(text: &str) -> Option<String> {
    for line in text.lines() {
        for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(path) = line.strip_prefix(marker) {
                let path = path.trim();
                if !path.is_empty() {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

/// A patch supplied in unified diff form.
fn patch_diff(input: &Value) -> Option<Diff> {
    let text = ["patch", "input", "diff", "content"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))?;
    let path = file_path(input).or_else(|| patch_file_path(text));
    let mut diff = Diff {
        path,
        ..Diff::default()
    };
    for line in text.lines() {
        if line.starts_with("+++ ") || line.starts_with("--- ") {
            continue;
        }
        if line.starts_with("@@") || line.starts_with("*** ") {
            continue;
        }
        let (kind, rest) = match line.strip_prefix('+') {
            Some(rest) => (DiffLineKind::Added, rest),
            None => match line.strip_prefix('-') {
                Some(rest) => (DiffLineKind::Removed, rest),
                None => (DiffLineKind::Context, line),
            },
        };
        diff.total_lines += 1;
        match kind {
            DiffLineKind::Added => diff.added += 1,
            DiffLineKind::Removed => diff.removed += 1,
            DiffLineKind::Context => {}
        }
        diff.lines.push(DiffLine {
            kind,
            text: rest.to_string(),
        });
    }
    if diff.lines.is_empty() {
        return None;
    }
    truncate(&mut diff);
    Some(diff)
}

/// The path a tool call acted on.
fn file_path(input: &Value) -> Option<String> {
    input
        .get("file_path")
        .or_else(|| input.get("filePath"))
        .or_else(|| input.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The difference between two texts, line by line.
fn diff_between(old: &str, new: &str) -> Diff {
    let diff = TextDiff::from_lines(old, new);
    let mut out = Diff::default();
    for change in diff.iter_all_changes() {
        let kind = match change.tag() {
            ChangeTag::Delete => DiffLineKind::Removed,
            ChangeTag::Insert => DiffLineKind::Added,
            ChangeTag::Equal => DiffLineKind::Context,
        };
        out.total_lines += 1;
        match kind {
            DiffLineKind::Added => out.added += 1,
            DiffLineKind::Removed => out.removed += 1,
            DiffLineKind::Context => {}
        }
        out.lines.push(DiffLine {
            kind,
            text: change.value().trim_end_matches('\n').to_string(),
        });
    }
    truncate(&mut out);
    out
}

/// Cut a diff down to the number of lines that are worth showing.
fn truncate(diff: &mut Diff) {
    if diff.lines.len() <= MAX_LINES {
        return;
    }
    let kept = diff.lines.len() / 2;
    let start = kept.min(diff.lines.len() - MAX_LINES);
    let end = (start + MAX_LINES).min(diff.lines.len());
    let mut lines = diff.lines[start..end].to_vec();
    lines.insert(
        0,
        DiffLine {
            kind: DiffLineKind::Context,
            text: format!("… {} lines hidden", start),
        },
    );
    lines.push(DiffLine {
        kind: DiffLineKind::Context,
        text: format!("… {} lines hidden", diff.lines.len() - end),
    });
    diff.lines = lines;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_write_shows_the_whole_file_as_added() {
        let diff = diff_for(
            "Write",
            &json!({"file_path": "/tmp/a.rs", "content": "one\ntwo\n"}),
        )
        .unwrap();
        assert_eq!(diff.path.as_deref(), Some("/tmp/a.rs"));
        assert_eq!(diff.added, 2);
        assert_eq!(diff.removed, 0);
        assert_eq!(diff.lines[0].kind, DiffLineKind::Added);
        assert_eq!(diff.lines[0].text, "one");
    }

    #[test]
    fn an_edit_shows_only_the_changed_lines() {
        let diff = diff_for(
            "Edit",
            &json!({
                "file_path": "/tmp/a.rs",
                "old_string": "fn main() {\n    println!(\"a\");\n}",
                "new_string": "fn main() {\n    println!(\"b\");\n}"
            }),
        )
        .unwrap();
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert!(
            diff.lines
                .iter()
                .any(|l| l.kind == DiffLineKind::Added && l.text.contains("\"b\""))
        );
    }

    #[test]
    fn a_tool_that_does_not_edit_a_file_has_no_diff() {
        assert!(diff_for("Bash", &json!({"command": "ls"})).is_none());
        assert!(diff_for("Read", &json!({"file_path": "/tmp/a.rs"})).is_none());
    }

    #[test]
    fn a_write_without_content_has_no_diff() {
        assert!(diff_for("Write", &json!({"file_path": "/tmp/a.rs"})).is_none());
    }

    #[test]
    fn a_patch_is_read_as_unified_diff() {
        let diff = diff_for(
            "apply_patch",
            &json!({"input": "--- a/x.rs\n+++ b/x.rs\n@@ -1 +1 @@\n-old\n+new\n"}),
        )
        .unwrap();
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert_eq!(diff.lines[0].kind, DiffLineKind::Removed);
        assert_eq!(diff.lines[1].kind, DiffLineKind::Added);
    }

    #[test]
    fn a_codex_patch_names_the_file_it_touches() {
        let patch = "*** Begin Patch\n*** Update File: /work/demo-app/src/routes.rs\n@@\n-old\n+new\n*** End Patch\n";
        let diff = diff_for("apply_patch", &json!({"input": patch})).unwrap();
        assert_eq!(diff.path.as_deref(), Some("/work/demo-app/src/routes.rs"));
        assert_eq!(diff.added, 1);
        assert_eq!(diff.removed, 1);
        assert!(diff.lines.iter().all(|l| !l.text.starts_with("***")));
        assert_eq!(
            patch_file_path(patch).as_deref(),
            Some("/work/demo-app/src/routes.rs")
        );
        assert_eq!(patch_file_path("no patch here"), None);
    }

    #[test]
    fn a_long_diff_is_cut_short() {
        let content = (0..1000)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let diff = diff_for("Write", &json!({"content": content})).unwrap();
        assert!(diff.is_truncated());
        assert!(diff.lines.len() <= MAX_LINES + 2);
        assert_eq!(diff.added, 1000);
    }

    #[test]
    fn text_form_carries_the_prefixes() {
        let diff = diff_for("Write", &json!({"content": "one\ntwo\n"})).unwrap();
        let text = diff.to_text();
        assert!(text.contains("+one"));
        assert!(text.contains("+two"));
    }
}
