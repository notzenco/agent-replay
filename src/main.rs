use anyhow::Result;
use clap::Parser;

use agent_replay::cli::{Cli, run};

fn main() -> Result<()> {
    let cli = Cli::parse();
    run(cli)
}
