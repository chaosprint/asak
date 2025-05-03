use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Sample, SampleFormat, SupportedStreamConfig};
use crossbeam::channel::{unbounded, Receiver};
use crossterm::event::{self, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use hound::{WavSpec, WavWriter};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::widgets::canvas::{Canvas, Circle, Line};
use ratatui::{
    prelude::{CrosstermBackend, Terminal},
    style::{Color, Style},
    text::Span,
    widgets::{Block, Borders, Gauge, Paragraph},
};
use std::f64::consts::PI;
use std::io::{stdout, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

fn calculate_level(samples: &[f32]) -> Vec<(f32, f32)> {
    let mut v = vec![];
    for frame in samples.chunks(2) {
        let square_sum: f32 = frame.iter().map(|&sample| (sample).powi(2)).sum();
        let mean: f32 = square_sum / frame.len() as f32;
        let rms = mean.sqrt();

        let peak = frame
            .iter()
            .map(|&sample| sample.abs())
            .max_by(|a, b| a.partial_cmp(b).unwrap());
        v.push((rms, peak.unwrap_or(0.0)));
    }
    v
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
    let mut last_audio_data = Vec::new();

    loop {
        let now = Instant::now();
        let duration = now.duration_since(start_time);
        let secs = duration.as_secs_f32();

        // Update angles for rotation
        angle1 = (angle1 + 0.1) % (2.0 * PI);
        angle2 = (angle2 + 0.15) % (2.0 * PI);

        // Process audio data for visualization
        while let Ok(data) = ui_rx.try_recv() {
            last_audio_data = data;
        }

        draw_rotating_discs(&mut terminal, secs, angle1, angle2, &last_audio_data)?;

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
    secs: f32,
    angle1: f64,
    angle2: f64,
    audio_data: &[f32],
) -> anyhow::Result<()> {
    // Check for zero area before drawing
    let size = terminal.size()?;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            [
                Constraint::Length(1),
                Constraint::Min(10),
                Constraint::Length(3),
                Constraint::Length(3),
            ]
            .as_ref(),
        )
        .split(Rect::new(0, 0, size.width, size.height));

    let canvas_rect = chunks[1];
    if canvas_rect.width == 0 || canvas_rect.height == 0 {
        // If the canvas area is zero, we can skip the terminal.draw call
        // or handle it gracefully, maybe just drawing the top/bottom parts
        // For now, let's just skip the entire draw operation for this frame
        return Ok(());
    }

    terminal.draw(|f| {
        // Recalculate chunks inside the closure as `f.size()` might differ slightly?
        // Or just use the previously calculated chunks. Let's reuse chunks.
        let size = f.area(); // Get size specific to this frame draw context
        let chunks = Layout::default() // Re-split based on frame size
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(1),
                    Constraint::Min(10),
                    Constraint::Length(3),
                    Constraint::Length(3),
                ]
                .as_ref(),
            )
            .split(size);

        // Top row with help text and time
        let top_row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(80), Constraint::Percentage(20)])
            .split(chunks[0]);

        // Help text on the left (yellow)
        let help_text = Paragraph::new(Span::styled(
            "press ENTER to stop and quit recorder",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        ))
        .alignment(Alignment::Left);
        f.render_widget(help_text, top_row[0]);

        // Time on the right (red)
        let time_text = format!("{:.1}", secs);
        let time_display = Paragraph::new(time_text)
            .style(Style::default().fg(Color::Red))
            .alignment(Alignment::Right);
        f.render_widget(time_display, top_row[1]);

        // --- Aspect Ratio Correction --- START
        let canvas_rect = chunks[1]; // Use chunks calculated inside closure
                                     // No need to check for zero size again, handled outside

        let canvas_width_chars = canvas_rect.width as f64;
        let canvas_height_chars = canvas_rect.height as f64;
        let char_aspect_ratio = 2.0; // Assume terminal char height is approx 2x width

        // Calculate the aspect ratio needed for the world coordinates to make circles appear round
        let canvas_aspect_ratio = canvas_height_chars / canvas_width_chars;
        let world_aspect_ratio = char_aspect_ratio * canvas_aspect_ratio;

        // Define the fixed horizontal range for world coordinates
        let x_world_range = 100.0;
        let x_bounds = [-x_world_range / 2.0, x_world_range / 2.0]; // [-50.0, 50.0]

        // Calculate the corresponding vertical range based on the desired world aspect ratio
        let y_world_range = x_world_range * world_aspect_ratio;
        let y_bounds = [-y_world_range / 2.0, y_world_range / 2.0];
        // --- Aspect Ratio Correction --- END

        // Define the circle parameters in world coordinates (using a fixed radius)
        let circle_radius = 15.0;
        let center1_x = -20.0;
        let center1_y = 0.0;
        let center2_x = 20.0;
        let center2_y = 0.0;

        // Create canvas with dynamically adjusted bounds
        let canvas = Canvas::default()
            .block(
                Block::default().borders(Borders::ALL), // .title("asak recorder"),
            )
            .x_bounds(x_bounds) // Use fixed x_bounds
            .y_bounds(y_bounds) // Use dynamically calculated y_bounds for aspect ratio correction
            .paint(move |ctx| {
                // Draw shapes using world coordinates; aspect ratio is handled by bounds

                // Draw the disc outlines
                ctx.draw(&Circle {
                    x: center1_x,
                    y: center1_y,
                    radius: circle_radius,
                    color: Color::White,
                });

                ctx.draw(&Circle {
                    x: center2_x,
                    y: center2_y,
                    radius: circle_radius,
                    color: Color::White,
                });

                // Draw inner circles
                ctx.draw(&Circle {
                    x: center1_x,
                    y: center1_y,
                    radius: circle_radius * 0.2,
                    color: Color::White,
                });

                ctx.draw(&Circle {
                    x: center2_x,
                    y: center2_y,
                    radius: circle_radius * 0.2,
                    color: Color::White,
                });

                // Draw rotating lines (use world radius directly)
                for i in 0..4 {
                    let line_angle = angle1 + (i as f64 * PI / 2.0);
                    let end_x = center1_x + circle_radius * line_angle.cos();
                    let end_y = center1_y + circle_radius * line_angle.sin();

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
                    let end_x = center2_x + circle_radius * line_angle.cos();
                    let end_y = center2_y + circle_radius * line_angle.sin();

                    ctx.draw(&Line {
                        x1: center2_x,
                        y1: center2_y,
                        x2: end_x,
                        y2: end_y,
                        color: Color::White,
                    });
                }

                // Draw grooves (use world radius)
                for r in 1..5 {
                    let groove_radius1 = circle_radius * (0.3 + r as f64 * 0.15);
                    let groove_radius2 = circle_radius * (0.3 + r as f64 * 0.15);

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

        // Add level meters
        let levels = calculate_level(audio_data);

        if !levels.is_empty() {
            // Ensure we have at least 2 channels (stereo)
            let left = if levels.len() > 0 { levels[0].0 } else { 0.0 };
            let right = if levels.len() > 1 { levels[1].0 } else { 0.0 };

            let db_left = if left > 0.0 {
                (20.0 * left.log10()) as i32
            } else {
                -90
            };
            let db_right = if right > 0.0 {
                (20.0 * right.log10()) as i32
            } else {
                -90
            };

            // Determine color based on level (red for clipping)
            let left_color = if left > 0.9 { Color::Red } else { Color::Green };
            let right_color = if right > 0.9 {
                Color::Red
            } else {
                Color::Green
            };

            let left_gauge = Gauge::default()
                .block(Block::new().title("Left dB").borders(Borders::ALL))
                .gauge_style(Style::default().fg(left_color))
                .label(Span::styled(
                    format!(
                        "{} dB",
                        match db_left {
                            x if x < -90 => "-inf".to_string(),
                            x => x.to_string(),
                        }
                    ),
                    Style::default()
                        .add_modifier(Modifier::ITALIC | Modifier::BOLD)
                        .fg(Color::White),
                ))
                .ratio(left as f64);

            let right_gauge = Gauge::default()
                .block(Block::new().title("Right dB").borders(Borders::ALL))
                .gauge_style(Style::default().fg(right_color))
                .label(Span::styled(
                    format!(
                        "{} dB",
                        match db_right {
                            x if x < -90 => "-inf".to_string(),
                            x => x.to_string(),
                        }
                    ),
                    Style::default()
                        .add_modifier(Modifier::ITALIC | Modifier::BOLD)
                        .fg(Color::White),
                ))
                .ratio(right as f64);

            f.render_widget(left_gauge, chunks[2]);
            f.render_widget(right_gauge, chunks[3]);
        }
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
