//! agent-replay: step through coding-agent session logs and export them.

pub mod cli;
pub mod diff;
pub mod discover;
pub mod export;
pub mod format;
pub mod honeytoken;
pub mod model;
pub mod redact;
pub mod stats;
pub mod tui;

/// The crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
