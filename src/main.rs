use anyhow::Result;
use clap::Parser;

mod cli;
mod tui;

use cli::Cli;

fn main() -> Result<()> {
    let _cli = Cli::parse();
    tui::run()
}
