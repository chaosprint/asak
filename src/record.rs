use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SupportedStreamConfig};
use crossbeam::channel::{unbounded, Receiver};
use crossterm::event::{self, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use hound::{WavSpec, WavWriter};
use ratatui::style::Modifier;
use ratatui::widgets::canvas::{Canvas, Circle, Line};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{CrosstermBackend, Terminal, Text},
    style::{Color, Style},
    text::Span,
    widgets::Paragraph,
    widgets::{Block, Borders},
};
use std::f64::consts::PI;
use std::io::{stdout, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

fn calculate_rms(samples: &[f32]) -> f64 {
    let square_sum: f64 = samples.iter().map(|&sample| (sample as f64).powi(2)).sum();
    let mean = square_sum / samples.len() as f64;
    mean.sqrt()
}

fn record_tui(ui_rx: Receiver<Vec<f32>>, is_recording: Arc<AtomicBool>) -> anyhow::Result<()> {
    let start_time = Instant::now();
    let refresh_interval = Duration::from_millis(100);

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    let mut angle1 = 0.0;
    let mut angle2 = 0.0;

    loop {
        let now = Instant::now();
        let duration = now.duration_since(start_time);
        let recording_time = format!("Recording Time: {:.2}s", duration.as_secs_f32());

        // Update angles for rotation
        angle1 = (angle1 + 0.1) % (2.0 * PI);
        angle2 = (angle2 + 0.15) % (2.0 * PI);

        // Always consume data from channel to avoid backlog
        while let Ok(_) = ui_rx.try_recv() {
            // Just consume the data, we don't need it for visualization
        }

        draw_rotating_discs(&mut terminal, recording_time, angle1, angle2)?;

        if event::poll(refresh_interval)? {
            if let event::Event::Key(event) = event::read()? {
                if event.code == KeyCode::Enter {
                    is_recording.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }
    }

    execute!(stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}

fn draw_rotating_discs(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    recording_time: String,
    angle1: f64,
    angle2: f64,
) -> anyhow::Result<()> {
    terminal.draw(|f| {
        let size = f.size();

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

        let block = Block::default().title("Recording").borders(Borders::NONE);
        let time_paragraph = Paragraph::new(Text::raw(&recording_time))
            .style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD));
        f.render_widget(block, chunks[0]);
        f.render_widget(time_paragraph, chunks[0]);

        let label = Span::styled(
            "press ENTER to exit tui and finish recording...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        );

        f.render_widget(Paragraph::new(label), chunks[2]);

        // Create canvas with two record discs
        let canvas = Canvas::default()
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("asak recorder"),
            )
            .x_bounds([-50.0, 50.0])
            .y_bounds([-25.0, 25.0])
            .paint(move |ctx| {
                // Left disc
                let center1_x = -20.0;
                let center1_y = 0.0;
                let radius1 = 15.0;

                // Right disc
                let center2_x = 20.0;
                let center2_y = 0.0;
                let radius2 = 15.0;

                // Draw the disc outlines
                ctx.draw(&Circle {
                    x: center1_x,
                    y: center1_y,
                    radius: radius1,
                    color: Color::White,
                });

                ctx.draw(&Circle {
                    x: center2_x,
                    y: center2_y,
                    radius: radius2,
                    color: Color::White,
                });

                // Draw inner circles
                ctx.draw(&Circle {
                    x: center1_x,
                    y: center1_y,
                    radius: radius1 * 0.2,
                    color: Color::White,
                });

                ctx.draw(&Circle {
                    x: center2_x,
                    y: center2_y,
                    radius: radius2 * 0.2,
                    color: Color::White,
                });

                // Draw rotating lines for left disc
                for i in 0..4 {
                    let line_angle = angle1 + (i as f64 * PI / 2.0);
                    let end_x = center1_x + radius1 * line_angle.cos();
                    let end_y = center1_y + radius1 * line_angle.sin();

                    ctx.draw(&Line {
                        x1: center1_x,
                        y1: center1_y,
                        x2: end_x,
                        y2: end_y,
                        color: Color::White,
                    });
                }

                // Draw rotating lines for right disc
                for i in 0..4 {
                    let line_angle = angle2 + (i as f64 * PI / 2.0);
                    let end_x = center2_x + radius2 * line_angle.cos();
                    let end_y = center2_y + radius2 * line_angle.sin();

                    ctx.draw(&Line {
                        x1: center2_x,
                        y1: center2_y,
                        x2: end_x,
                        y2: end_y,
                        color: Color::White,
                    });
                }

                // Draw grooves on the discs
                for r in 1..5 {
                    let groove_radius1 = radius1 * (0.3 + r as f64 * 0.15);
                    let groove_radius2 = radius2 * (0.3 + r as f64 * 0.15);

                    ctx.draw(&Circle {
                        x: center1_x,
                        y: center1_y,
                        radius: groove_radius1,
                        color: Color::Gray,
                    });

                    ctx.draw(&Circle {
                        x: center2_x,
                        y: center2_y,
                        radius: groove_radius2,
                        color: Color::Gray,
                    });
                }
            });

        f.render_widget(canvas, chunks[1]);
    })?;
    Ok(())
}

pub fn record_audio(output: String, device: Option<u8>, jack: bool) -> anyhow::Result<()> {
    let output = format!("{}.wav", output.replace(".wav", ""));
    let (ui_tx, ui_rx) = unbounded();
    let (writer_tx, writer_rx) = unbounded();
    let is_recording = Arc::new(AtomicBool::new(true));
    let is_recording_for_thread = is_recording.clone();

    #[cfg(all(
        any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        ),
        feature = "jack"
    ))]
    let host = if jack {
        cpal::host_from_id(cpal::available_hosts()
            .into_iter()
            .find(|id| *id == cpal::HostId::Jack)
            .expect(
                "make sure --features jack is specified. only works on OSes where jack is available",
            )).expect("jack host unavailable")
    } else {
        cpal::default_host()
    };

    #[cfg(any(
        not(any(
            target_os = "linux",
            target_os = "dragonfly",
            target_os = "freebsd",
            target_os = "netbsd"
        )),
        not(feature = "jack")
    ))]
    assert!(
        !jack,
        "jack is only supported on linux, dragonfly, freebsd, and netbsd"
    );
    let host = cpal::default_host();

    let device = if device.is_none() {
        host.default_input_device()
    } else if let Some(index) = device {
        host.input_devices()?.nth(index as usize)
    } else {
        panic!("failed to find output device")
    }
    .expect("failed to find output device");

    let config = device.default_input_config().unwrap();
    let o = output.to_owned();
    let spec = wav_spec_from_config(&device.default_input_config().unwrap());

    let recording_thread = std::thread::spawn(move || {
        let err_fn = move |err| eprintln!("an error occurred on stream: {}", err);
        let stream = match config.sample_format() {
            cpal::SampleFormat::I8 => device.build_input_stream(
                &config.into(),
                move |data: &[i8], _: &_| {
                    let float_data: Vec<f32> = data
                        .iter()
                        .map(|&sample| sample.to_float_sample())
                        .collect();
                    ui_tx.send(float_data.clone()).ok();
                    writer_tx.send(float_data).ok();
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &_| {
                    let float_data: Vec<f32> = data
                        .iter()
                        .map(|&sample| sample.to_float_sample())
                        .collect();
                    ui_tx.send(float_data.clone()).ok();
                    writer_tx.send(float_data).ok();
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I32 => device.build_input_stream(
                &config.into(),
                move |data: &[i32], _: &_| {
                    let float_data: Vec<f32> = data
                        .iter()
                        .map(|&sample| sample.to_float_sample())
                        .collect();
                    ui_tx.send(float_data.clone()).ok();
                    writer_tx.send(float_data).ok();
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &_| {
                    let float_data: Vec<f32> = data.to_vec();
                    ui_tx.send(float_data.clone()).ok();
                    writer_tx.send(float_data).ok();
                },
                err_fn,
                None,
            )?,
            sample_format => {
                return Err(anyhow::Error::msg(format!(
                    "Unsupported sample format '{sample_format}'"
                )))
            }
        };
        stream.play()?;

        while is_recording_for_thread.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        stream.pause()?;
        Ok(())
    });

    let writer_thread = std::thread::spawn(move || -> anyhow::Result<()> {
        let path = std::path::Path::new(&o);

        let spec2 = WavSpec {
            channels: spec.channels,
            sample_rate: spec.sample_rate,
            bits_per_sample: spec.bits_per_sample,
            sample_format: hound::SampleFormat::Float,
        };

        let mut writer = WavWriter::create(path, spec2).unwrap();

        while let Ok(data) = writer_rx.recv() {
            for sample in data {
                writer.write_sample(sample).ok();
            }
        }

        writer.finalize().unwrap();
        Ok(())
    });

    record_tui(ui_rx, is_recording.clone())?;
    is_recording.store(false, Ordering::SeqCst);
    recording_thread.join().unwrap()?;
    writer_thread.join().unwrap()?;

    Ok(())
}

fn wav_spec_from_config(config: &SupportedStreamConfig) -> WavSpec {
    WavSpec {
        channels: config.channels() as _,
        sample_rate: config.sample_rate().0 as _,
        bits_per_sample: (config.sample_format().sample_size() * 8) as _,
        sample_format: if config.sample_format() == SampleFormat::F32 {
            hound::SampleFormat::Float
        } else {
            hound::SampleFormat::Int
        },
    }
}
