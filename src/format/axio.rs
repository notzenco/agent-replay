//! Reads Axio session logs.
//!
//! A session is an append-only JSONL file under `~/.axio/agent/sessions/`.
//! The first line is a header; every following line holds one message.

use std::path::Path;

use anyhow::Result;

use crate::format::SessionReader;
use crate::model::Session;

/// Marker type so `SessionReader` can be implemented for this format.
pub struct Axio;

impl SessionReader for Axio {
    fn read(path: &Path) -> Result<Session> {
        read_session(path)
    }

    fn detect(path: &Path) -> bool {
        detect_path(path)
    }
}

/// Read an Axio session log.
pub fn read_session(_path: &Path) -> Result<Session> {
    anyhow::bail!("axios session reading is not implemented yet")
}

/// Whether a path looks like an Axio session log.
pub fn detect_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.contains(".axio/agent/sessions")
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e == "jsonl")
}
