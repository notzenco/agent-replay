//! Reads Codex rollout logs.
//!
//! A rollout is an append-only JSONL file under `~/.codex/sessions/`. Every
//! line is `{ "timestamp": …, "type": …, "payload": { … } }`.

use std::path::Path;

use anyhow::Result;

use crate::format::SessionReader;
use crate::model::Session;

/// Marker type so `SessionReader` can be implemented for this format.
pub struct Codex;

impl SessionReader for Codex {
    fn read(path: &Path) -> Result<Session> {
        read_session(path)
    }

    fn detect(path: &Path) -> bool {
        detect_path(path)
    }
}

/// Read a Codex rollout log.
pub fn read_session(_path: &Path) -> Result<Session> {
    anyhow::bail!("codex rollout reading is not implemented yet")
}

/// Whether a path looks like a Codex rollout log.
pub fn detect_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains(".codex/sessions")
        && path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("rollout-"))
}
