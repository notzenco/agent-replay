//! The command line interface.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::discover;
use crate::export::{self, ExportOptions};
use crate::model::SessionSource;
use crate::stats;

/// Step through coding-agent session logs and export them as HTML.
#[derive(Debug, Parser)]
#[command(name = "agent-replay", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// What to do.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List the session logs found on this machine.
    List(ListArgs),
    /// Open a session in the terminal viewer.
    View(ViewArgs),
    /// Show what a session did.
    Stats(StatsArgs),
    /// Write a session out as a self-contained HTML timeline.
    Export(ExportArgs),
}

/// Arguments of `agent-replay list`.
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only list sessions from this format.
    #[arg(long, value_enum)]
    pub format: Option<FormatArg>,
    /// Only list sessions from this working directory.
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Show the sessions as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Arguments of `agent-replay view`.
#[derive(Debug, Args)]
pub struct ViewArgs {
    /// The session: a path to a log, or a session id.
    pub session: String,
}

/// Arguments of `agent-replay stats`.
#[derive(Debug, Args)]
pub struct StatsArgs {
    /// The session: a path to a log, or a session id.
    pub session: String,
    /// Show the statistics as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Arguments of `agent-replay export`.
#[derive(Debug, Args)]
pub struct ExportArgs {
    /// The session: a path to a log, or a session id.
    pub session: String,
    /// Where to write the HTML file.
    #[arg(short, long)]
    pub output: PathBuf,
    /// Leave secrets, home paths and email addresses in the file.
    #[arg(long)]
    pub no_redact: bool,
    /// Do not highlight agent-tripwire honeytokens.
    #[arg(long)]
    pub no_highlight_honeytokens: bool,
}

/// The format of a session log, as a command line value.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FormatArg {
    /// Claude Code session logs.
    Claude,
    /// Codex rollout logs.
    Codex,
    /// Axio session logs.
    Axio,
}

impl From<FormatArg> for SessionSource {
    fn from(value: FormatArg) -> Self {
        match value {
            FormatArg::Claude => SessionSource::ClaudeCode,
            FormatArg::Codex => SessionSource::Codex,
            FormatArg::Axio => SessionSource::Axio,
        }
    }
}

/// Run the command line interface.
pub fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::List(args) => list(args),
        Command::View(args) => crate::tui::view(&resolve(&args.session)?),
        Command::Stats(args) => stats_command(args),
        Command::Export(args) => export_command(args),
    }
}

/// List the sessions found on this machine.
fn list(args: ListArgs) -> Result<()> {
    let sources: Vec<SessionSource> = match args.format {
        Some(format) => vec![format.into()],
        None => SessionSource::all().to_vec(),
    };
    let found = discover::discover(&sources)?;

    let found: Vec<_> = found
        .into_iter()
        .filter(|entry| match &args.cwd {
            Some(wanted) => {
                let wanted = wanted.to_string_lossy();
                entry.path.to_string_lossy().contains(wanted.as_ref())
            }
            None => true,
        })
        .collect();

    if args.json {
        let entries: Vec<serde_json::Value> = found
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "source": entry.source.as_str(),
                    "path": entry.path.display().to_string(),
                    "size": entry.size,
                    "modified": entry.modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(entries))?
        );
        return Ok(());
    }

    if found.is_empty() {
        println!("no sessions found");
        return Ok(());
    }

    let widest = found
        .iter()
        .map(|entry| entry.id.len())
        .max()
        .unwrap_or(0)
        .max(4);
    for entry in found {
        let size = human_size(entry.size);
        println!(
            "{:<widest$}  {:<11}  {:>9}  {}",
            entry.id,
            entry.source.as_str(),
            size,
            entry.path.display()
        );
    }
    Ok(())
}

/// Show what a session did.
fn stats_command(args: StatsArgs) -> Result<()> {
    let path = resolve(&args.session)?;
    let session = crate::format::read(&path)?;
    let stats = stats::compute(&session);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&stats::to_json(&stats))?);
    } else {
        print!("{}", stats::to_text(&stats));
    }
    Ok(())
}

/// Write a session out as an HTML timeline.
fn export_command(args: ExportArgs) -> Result<()> {
    let path = resolve(&args.session)?;
    let session = crate::format::read(&path)?;
    let options = ExportOptions {
        redact: !args.no_redact,
        highlight_honeytokens: !args.no_highlight_honeytokens,
    };
    let html = export::to_html(&session, &options);

    if let Some(parent) = args.output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating the directory {}", parent.display()))?;
    }
    std::fs::write(&args.output, html)
        .with_context(|| format!("writing {}", args.output.display()))?;

    let mut summary = format!("wrote {}", args.output.display());
    if options.redact {
        summary.push_str(" (redacted)");
    } else {
        summary.push_str(" (not redacted)");
    }
    println!("{summary}");
    Ok(())
}

/// Resolve a session reference to a file.
fn resolve(reference: &str) -> Result<PathBuf> {
    discover::resolve(reference)
}

/// A file size in a form a person can read.
fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
