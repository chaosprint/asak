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
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Record an audio file
    Rec(RecArgs),
    /// Play an audio file
    Play(PlayArgs),
}

// / Process an audio file with effects
// Proc(ProcArgs),

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

/// Arguments used for the `proc` command
#[derive(Args, Debug)]
struct ProcArgs {
    /// Input audio file path
    #[arg(required = true)]
    input: String,

    /// Output audio file path
    #[arg(required = true)]
    output: String,

    /// Apply a resonant low-pass filter with specified cutoff frequency and resonance (Q)
    #[arg(long, num_args = 2)]
    rlpf: Option<Vec<f32>>, // No longer optional

    /// Apply a resonant high-pass filter with specified cutoff frequency and resonance (Q)
    #[arg(long)]
    rhpf: Option<Vec<f32>>,
}

fn main() {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Rec(args) => {
            // Recording logic with args.output
            // println!("Recording to {}", args.output);
            record_audio(&args.output).unwrap();
        }
        Commands::Play(args) => {
            // Playback logic with args.input
            // println!("Playing {}", args.input);
            play_audio(&args.input).unwrap();
        } // Commands::Proc(args) => {
          //     // Audio processing logic with args.input, args.output
          //     println!("Processing {} to {}", args.input, args.output);
          //     if let Some(rlpf) = &args.rlpf {
          //         println!(
          //             "Applying resonant low-pass filter with
          //             cutoff frequency: {} and resonance: {}",
          //             rlpf[0], rlpf[1]
          //         );
          //     }
          // }
    }
}
