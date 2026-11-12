//! agent-replay: step through coding-agent session logs and export them.

pub mod diff;
pub mod format;
pub mod honeytoken;
pub mod model;
pub mod redact;

/// The crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
