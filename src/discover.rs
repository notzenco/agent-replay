//! Finds session logs on this machine.
//!
//! A session can be named by its path or by its id. Ids are resolved against
//! the places the agents keep their logs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Result, bail};

use crate::model::SessionSource;

/// Where the agents keep their session logs.
pub struct Roots {
    pub claude: PathBuf,
    pub codex: PathBuf,
    pub axio: PathBuf,
}

impl Roots {
    /// The roots for the current user.
    pub fn for_user() -> Roots {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Roots {
            claude: home.join(".claude").join("projects"),
            codex: home.join(".codex").join("sessions"),
            axio: home.join(".axio").join("agent").join("sessions"),
        }
    }

    /// The root for one source.
    pub fn for_source(&self, source: SessionSource) -> &Path {
        match source {
            SessionSource::ClaudeCode => &self.claude,
            SessionSource::Codex => &self.codex,
            SessionSource::Axio => &self.axio,
        }
    }
}

/// A session log found on disk.
#[derive(Debug, Clone)]
pub struct Discovered {
    pub id: String,
    pub source: SessionSource,
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    pub size: u64,
}

/// Find every session log, newest first.
pub fn discover(sources: &[SessionSource]) -> Result<Vec<Discovered>> {
    let roots = Roots::for_user();
    let mut found = Vec::new();
    for source in sources {
        let root = roots.for_source(*source);
        if !root.exists() {
            continue;
        }
        let mut entries = Vec::new();
        collect(root, &mut entries, 0);
        found.extend(entries);
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id)));
    Ok(found)
}

/// Work out the format of a file from where it lives.
fn source_of(path: &Path) -> Option<SessionSource> {
    let text = path.to_string_lossy();
    if text.contains(".claude/projects") {
        return Some(SessionSource::ClaudeCode);
    }
    if text.contains(".codex/sessions") {
        return Some(SessionSource::Codex);
    }
    if text.contains(".axio/agent/sessions") {
        return Some(SessionSource::Axio);
    }
    None
}

/// Whether a file inside an agent's session tree is a session log at all.
///
/// Claude Code also writes workflow journals (`journal.jsonl`) into the same
/// tree. They are not session transcripts, so they are left out of the list
/// instead of failing when opened.
fn is_session_file(source: SessionSource, path: &Path) -> bool {
    match source {
        SessionSource::ClaudeCode => path.file_stem().and_then(|s| s.to_str()) != Some("journal"),
        SessionSource::Codex | SessionSource::Axio => true,
    }
}

/// Walk a directory, collecting the session logs inside it.
fn collect(dir: &Path, out: &mut Vec<Discovered>, depth: usize) {
    if depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out, depth + 1);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let Some(file_source) = source_of(&path) else {
            continue;
        };
        if !is_session_file(file_source, &path) {
            continue;
        }
        let metadata = entry.metadata().ok();
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        out.push(Discovered {
            id,
            source: file_source,
            path: path.clone(),
            modified: metadata.as_ref().and_then(|m| m.modified().ok()),
            size: metadata.as_ref().map(|m| m.len()).unwrap_or(0),
        });
    }
}

/// Resolve a session reference to a file.
///
/// A reference is either a path to a log or the id of one. An id matches
/// exactly, or as a prefix when that prefix is unique.
pub fn resolve(reference: &str) -> Result<PathBuf> {
    let path = PathBuf::from(reference);
    if path.is_file() {
        return Ok(path);
    }
    if path.is_dir() {
        bail!("{} is a directory, not a session log", path.display());
    }

    let found = discover(&SessionSource::all())?;
    let mut exact = Vec::new();
    let mut prefix = Vec::new();
    for candidate in found {
        if candidate.id == reference {
            exact.push(candidate.path);
        } else if candidate.id.starts_with(reference) {
            prefix.push(candidate.path);
        }
    }

    if exact.len() == 1 {
        return Ok(exact.remove(0));
    }
    if exact.len() > 1 {
        bail!("more than one session is called {reference}; pass the path to the one you mean");
    }
    if prefix.len() == 1 {
        return Ok(prefix.remove(0));
    }
    if prefix.len() > 1 {
        bail!(
            "{reference} matches {} sessions; pass more of the id or the full path",
            prefix.len()
        );
    }

    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"));
    bail!(
        "no session called {reference}; looked in {} and in the paths under {}",
        home.join(".claude").join("projects").display(),
        home.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workflow_journal_is_not_a_session_file() {
        let journal =
            Path::new("/home/user/.claude/projects/p/s/subagents/workflows/wf_1/journal.jsonl");
        assert!(!is_session_file(SessionSource::ClaudeCode, journal));
    }

    #[test]
    fn claude_session_and_subagent_logs_are_session_files() {
        let session =
            Path::new("/home/user/.claude/projects/p/9a07af0a-cb40-448d-9e43-78b4be13062e.jsonl");
        let subagent =
            Path::new("/home/user/.claude/projects/p/s/subagents/agent-a52716d450c7811eb.jsonl");
        assert!(is_session_file(SessionSource::ClaudeCode, session));
        assert!(is_session_file(SessionSource::ClaudeCode, subagent));
        assert!(is_session_file(
            SessionSource::Codex,
            Path::new("/home/user/.codex/sessions/2026/01/01/rollout-x.jsonl")
        ));
        assert!(is_session_file(
            SessionSource::Axio,
            Path::new("/home/user/.axio/agent/sessions/p/1-2.jsonl")
        ));
    }
}
