use clap::Parser;

/// Audio Swiss Army knife written in Rust. Like Sox but interactive with TUI.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {}
