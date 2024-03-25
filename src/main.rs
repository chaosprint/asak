use clap::{Args, Parser, Subcommand};

mod record;
use record::record_audio;

mod playback;
use playback::play_audio;

/// Audio Swiss Army knife written in Rust. Like Sox but interative with TUI.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// The audio device to use
    #[arg(short, long, default_value_t = String::from("default"))]
    device: String,

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
    jack: bool,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Record an audio file
    Rec(RecArgs),
    /// Play an audio file
    Play(PlayArgs),
}

/// Arguments used for the `rec` command
#[derive(Args, Debug)]
struct RecArgs {
    /// Path for the output audio file, e.g. `output`
    #[arg(required = false)]
    output: Option<String>,
}

/// Arguments used for the `play` command
#[derive(Args, Debug)]
struct PlayArgs {
    /// Path to the audio file to play; must be wav format for now, e.g. `input.wav`
    #[arg(required = true)]
    input: String,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Rec(args) => {
            // Pass the respective JACK usage flag to play_audio based on compile-time detection
            #[cfg(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd"
                ),
                feature = "jack"
            ))]
            {
                // If we're on the right platform and JACK is enabled, pass true to use JACK for playback
                record_audio(&args.input, &cli.device, &cli.jack).unwrap();
            }
            #[cfg(not(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd"
                ),
                feature = "jack"
            )))]
            {
                // If JACK is not available or the platform is unsupported, pass false to not use JACK
                record_audio(&args.output, &cli.device, false).unwrap();
            }
        }
        Commands::Play(args) => {
            // Pass the respective JACK usage flag to play_audio based on compile-time detection
            #[cfg(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd"
                ),
                feature = "jack"
            ))]
            {
                // If we're on the right platform and JACK is enabled, pass true to use JACK for playback
                play_audio(&args.input, &cli.device, &cli.jack).unwrap();
            }
            #[cfg(not(all(
                any(
                    target_os = "linux",
                    target_os = "dragonfly",
                    target_os = "freebsd",
                    target_os = "netbsd"
                ),
                feature = "jack"
            )))]
            {
                // If JACK is not available or the platform is unsupported, pass false to not use JACK
                play_audio(&args.input, &cli.device, false).unwrap();
            }
        }
    }
}
