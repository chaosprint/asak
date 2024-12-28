use clap::{Args, Parser, Subcommand};

/// Audio Swiss Army knife written in Rust. Like Sox but interactive with TUI.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// The audio device index to use
    #[arg(short, long)]
    pub device: Option<u8>,

    /// Use the JACK host
    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ),
        feature = "jack"
    ))]
    #[arg(short, long)]
    #[allow(dead_code)]
    pub jack: bool,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Record an audio file
    Rec(RecArgs),
    /// Play an audio file
    Play(PlayArgs),
    /// Monitor audio input with scopes
    Monitor(MonitorArgs),
    /// List available audio devices
    List,
}

/// Arguments used for the `rec` command
#[derive(Args, Debug)]
pub struct RecArgs {
    /// Path for the output audio file, e.g. `output`
    #[arg(required = false)]
    pub output: Option<String>,
}

/// Arguments used for the `play` command
#[derive(Args, Debug)]
pub struct PlayArgs {
    /// Path to the audio file to play; must be wav format for now, e.g. `input.wav`
    #[arg(required = false)]
    pub input: Option<String>,
}

/// Arguments used for the `monitor` command
#[derive(Args, Debug)]
pub struct MonitorArgs {
    /// Buffer size for the audio input monitoring, defaults to 1024, the higher the value the more latency
    #[arg(required = false, short, long)]
    pub buffer_size: Option<usize>,
}
