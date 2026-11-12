//! agent-replay: step through coding-agent session logs and export them.

pub mod format;
pub mod model;

/// The crate version, from `Cargo.toml`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
