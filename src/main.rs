use clap::Parser;
use colored::*;

mod record;
use cpal::traits::{DeviceTrait, HostTrait};
use inquire::{InquireError, Select, Text};
use record::record_audio;

mod playback;
use playback::play_audio;

mod monitor;
use monitor::start_monitoring;

mod cli;
use cli::{Cli, Commands};

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

                match &args.output {
                    Some(output) => {
                        record_audio(output.clone(), args.device, false).unwrap();
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
                            Ok(output) => record_audio(output, args.device, cli.jack).unwrap(),
                            Err(_) => println!("Recording cancelled."),
                        }
                    }
                };
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
                        record_audio(output.clone(), args.device, false).unwrap();
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
                            Ok(output) => record_audio(output, args.device, false).unwrap(),
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
                match &args.input {
                    Some(input) => play_audio(input, args.device, false).unwrap(),
                    None => {
                        let mut options: Vec<String> = vec![];
                        // check current directory for wav files
                        let files = std::fs::read_dir(".").unwrap();
                        for file in files {
                            let file = file.unwrap();
                            let path = file.path().clone();
                            let path = path.to_str().unwrap();
                            if path.ends_with(".wav") {
                                options.push(path.into());
                            }
                        }
                        if options.is_empty() {
                            println!("No wav files found in current directory");
                        } else {
                            let ans: Result<String, InquireError> =
                                Select::new("Select a wav file to play", options).prompt();
                            match ans {
                                Ok(input) => play_audio(&input, args.device, cli.jack).unwrap(),
                                Err(_) => println!("Playback cancelled."),
                            }
                        }
                    }
                }
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
                    Some(input) => play_audio(input, args.device, false).unwrap(),
                    None => {
                        let mut options: Vec<String> = vec![];
                        // check current directory for wav files
                        let files = std::fs::read_dir(".").unwrap();
                        for file in files {
                            let file = file.unwrap();
                            let path = file.path().clone();
                            let path = path.to_str().unwrap();
                            if path.ends_with(".wav") {
                                options.push(path.into());
                            }
                        }
                        if options.is_empty() {
                            println!("No wav files found in current directory");
                        } else {
                            let ans: Result<String, InquireError> =
                                Select::new("Select a wav file to play", options).prompt();
                            match ans {
                                Ok(input) => play_audio(&input, args.device, false).unwrap(),
                                Err(_) => println!("Playback cancelled."),
                            }
                        }
                    }
                }
            }
        }
        Commands::Monitor(args) => {
            let buffer_size = args.buffer_size.unwrap_or(1024);
            start_monitoring(buffer_size).unwrap();
        }
        Commands::List => {
            let host = cpal::default_host();
            let in_devices = host.input_devices().unwrap();
            let out_devices = host.output_devices().unwrap();

            println!("\n{}", "Available Audio Devices".bold().underline());
            println!("\n{}", "Usage:".yellow());
            println!(
                "  Recording: {} {}",
                "asak rec --device".bright_black(),
                "<index>".cyan()
            );
            println!(
                "  Playback: {} {}",
                "asak play --device".bright_black(),
                "<index>".cyan()
            );

            println!("\n{}", "=== Input Devices ===".green().bold());
            for (index, device) in in_devices.enumerate() {
                println!("#{}: {}", index.to_string().cyan(), device.name().unwrap());
            }

            println!("\n{}", "=== Output Devices ===".blue().bold());
            for (index, device) in out_devices.enumerate() {
                println!("#{}: {}", index.to_string().cyan(), device.name().unwrap());
            }

            println!(
                "\n{}",
                "Note: If no device is specified, the system default will be used.".italic()
            );
            println!();
        }
    }
}
