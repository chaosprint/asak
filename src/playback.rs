use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::event::{self, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use hound::WavReader;
use ratatui::style::Modifier;
use ratatui::symbols;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, Paragraph};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{CrosstermBackend, Terminal},
    style::{Color, Style},
};

use std::io::stdout;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub fn play_audio(file_path: &str) -> Result<()> {
    let host = cpal::default_host();
    let output_device = host
        .default_output_device()
        .expect("No output device available");
    let config = output_device
        .default_output_config()
        .expect("Failed to get default output config");

    let sys_chan = config.channels() as usize;

    let mut reader = WavReader::open(file_path)?;
    let spec = reader.spec();
    let num_channels = spec.channels as usize;
    let mut file_data: Vec<Vec<f32>> = vec![];

    for _ in 0..num_channels {
        file_data.push(Vec::new());
    }

    match spec.sample_format {
        hound::SampleFormat::Float => {
            let mut channel_index = 0;
            for result in reader.samples::<f32>() {
                let sample = result?;
                file_data[channel_index].push(sample);
                channel_index = (channel_index + 1) % num_channels;
            }
        }
        // Add other sample formats as necessary
        _ => unimplemented!(),
    }

    if sys_chan == 2 && num_channels == 1 {
        file_data.push(file_data[0].clone());
    }

    let file_data_clone = file_data.clone();
    let length = file_data[0].len();

    let sample_format = config.sample_format();
    // let sample_rate = cpal::SampleRate(spec.sample_rate);
    // let channels = spec.channels as u16;

    let pointer = Arc::new(AtomicUsize::new(0));

    // let data_arc = Arc::new(Mutex::new(data));

    // let file_clone = data_arc.clone();
    // let mut data_iter = data_arc.lock().unwrap().iter_mut();

    let err_fn = |err| eprintln!("an error occurred on the output stream: {}", err);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => output_device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(2) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < file_data.len() {
                            data[i + j] = file_data[j][p];
                        }
                    }

                    let next = if p + 1 < length { p + 1 } else { 0 };
                    pointer.store(next, std::sync::atomic::Ordering::Relaxed);
                }
            },
            err_fn,
            None,
        )?,
        // Handle other sample formats as needed
        _ => unimplemented!("This sample format is not yet implemented."),
    };
    stream.play()?;

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let start_time = Instant::now();
    let file_duration = WavReader::open(file_path)?.duration() as f32 / spec.sample_rate as f32;

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let event::Event::Key(event) = event::read()? {
                if event.code == KeyCode::Esc {
                    break;
                }
            }
        }

        let elapsed = start_time.elapsed().as_secs_f32();
        if elapsed >= file_duration {
            break; // Stop when the file duration is reached
        }

        let progress = elapsed / file_duration;

        terminal.draw(|f| {
            let size = f.size();
            let width = size.width as usize;

            // data vec is calculated here, pick width samples from the file data
            let mut data_vec: Vec<(f64, f64)> = vec![];
            for i in 0..width {
                let index = (i as f32 / width as f32 * length as f32) as usize;
                let rms = file_data_clone[0][index];
                data_vec.push((i as f64, rms as f64));
            }
            //     .enumerate()
            //     .filter_map(|(i, &x)| {
            //         if i % (length / width) == 0 {
            //             Some((i as f64, x as f64))
            //         } else {
            //             None
            //         }
            //     })
            //     .collect();

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Percentage(10),
                        Constraint::Percentage(80),
                        Constraint::Min(4),
                    ]
                    .as_ref(),
                )
                .split(size);
            let gauge = Gauge::default()
                .block(
                    Block::default()
                        .title(format!("PLAYBACK  {:.2}s/{:.2}s", elapsed, file_duration))
                        .borders(Borders::NONE),
                )
                .gauge_style(Style::default().fg(Color::Blue).bg(Color::Black))
                .percent((progress * 100.0) as u16);
            // f.render_widget(gauge, size);
            f.render_widget(gauge, chunks[0]);
            let datasets = vec![Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red))
                .data(&data_vec)];

            let chart = Chart::new(datasets)
                .x_axis(
                    Axis::default()
                        // .title("X Axis")
                        .style(Style::default().fg(Color::Gray))
                        // .labels(x_labels)
                        .bounds([0., width as f64]),
                )
                .y_axis(
                    Axis::default()
                        // .title("RMS")
                        .style(Style::default().fg(Color::Gray))
                        .bounds([-1.0, 1.]),
                );
            f.render_widget(chart, chunks[1]);
            let label = Span::styled(
                format!("press esc to exit tui and stop playback."),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC | Modifier::BOLD),
            );

            f.render_widget(Paragraph::new(label), chunks[2]);
        })?;
    }

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}

// fn playback_tui() {}
