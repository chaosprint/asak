use clap::{Args, Parser, Subcommand};

mod record;
use inquire::{InquireError, Select, Text};
use record::record_audio;

mod playback;
use playback::play_audio;

mod monitor;
use monitor::start_monitoring;

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
    /// Monitor audio input with scopes
    Monitor,
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
    #[arg(required = false)]
    input: Option<String>,
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
                match &args.output {
                    Some(output) => {
                        // let output = o;
                        record_audio(output.clone(), &cli.device, false).unwrap();
                    }
                    None => {
                        let now = chrono::Utc::now();
                        let name = format!(
                            "{}.wav",
                            now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                        );
                        // let output = Text::new("What is your name?").placeholder(name).prompt();
                        let output = Text {
                            initial_value: Some(&name),
                            ..Text::new("Please enter the output wav file name:")
                        }
                        .prompt();
                        match output {
                            Ok(output) => record_audio(output, &cli.device, false).unwrap(),
                            Err(_) => println!("Recording cancelled."),
                        }
                    }
                };
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
                match &args.input {
                    Some(input) => play_audio(input, &cli.device, false).unwrap(),
                    None => {
                        let mut options: Vec<String> = vec![];
                        // check current directory for wav files
                        let files = std::fs::read_dir(".").unwrap();
                        for file in files {
                            let file = file.unwrap();
                            let path = file.path().clone();
                            let path = path.to_str().unwrap();
                            if path.ends_with(".wav") {
                                options.push(format!("{}", path));
                            }
                        }
                        if options.is_empty() {
                            println!("No wav files found in current directory");
                        } else {
                            let ans: Result<String, InquireError> =
                                Select::new("Select a wav file to play", options).prompt();
                            match ans {
                                Ok(input) => play_audio(&input, &cli.device, false).unwrap(),
                                Err(_) => println!("Playback cancelled."),
                            }
                        }
                    }
                }
            }
        }
        Commands::Monitor => {
            start_monitoring().unwrap();
        }
    }
}
