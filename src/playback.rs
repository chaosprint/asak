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
use ratatui::widgets::canvas::{Canvas, Circle, Line};
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, Gauge, GraphType, Paragraph};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{CrosstermBackend, Terminal},
    style::{Color, Style},
};

use dasp_interpolate::linear::Linear;
use dasp_signal::Signal;
use std::f64::consts::PI;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[allow(unused_variables)]
pub fn play_audio(file_path: &str, device: Option<u8>, jack: bool) -> Result<()> {
    // Conditionally compile with jack if the feature is specified.
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
    let host = cpal::default_host();

    let device = if device.is_none() {
        host.default_output_device()
    } else if let Some(index) = device {
        host.output_devices()?.nth(index as usize)
    } else {
        panic!("failed to find output device");
    }
    .expect("failed to find output device");

    let config = device.default_output_config().unwrap();

    let sys_chan = config.channels() as usize;
    let sys_sr = config.sample_rate().0 as f64;
    let mut reader = WavReader::open(file_path).expect("failed to open wav file");
    let spec = reader.spec();
    let source_sr = spec.sample_rate as f64;

    let num_channels = spec.channels as usize;
    let bit = spec.bits_per_sample as usize;
    let mut file_data: Vec<Vec<f32>> = vec![];

    for _ in 0..num_channels {
        file_data.push(Vec::new());
    }

    let mut sample_count = 0;

    match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => {
                for result in reader.samples::<i16>() {
                    let sample = result? as f32 / i16::MAX as f32;
                    let channel = sample_count % num_channels;
                    file_data[channel].push(sample);
                    sample_count += 1;
                }
            }

            24 => {
                for result in reader.samples::<i32>() {
                    let sample = result?;
                    let sample = if sample & (1 << 23) != 0 {
                        (sample | !0xff_ffff) as f32
                    } else {
                        sample as f32
                    };
                    let sample = sample / (1 << 23) as f32;
                    let channel = sample_count % num_channels;
                    file_data[channel].push(sample);
                    sample_count += 1;
                }
            }

            32 => {
                for result in reader.samples::<i32>() {
                    let sample = result? as f32 / i32::MAX as f32;
                    let channel = sample_count % num_channels;
                    file_data[channel].push(sample);
                    sample_count += 1;
                }
            }
            _ => panic!("unsupported bit depth"),
        },
        hound::SampleFormat::Float => {
            for result in reader.samples::<f32>() {
                let sample = result?;
                let channel = sample_count % num_channels;
                file_data[channel].push(sample);
                sample_count += 1;
            }
        }
    }

    // TODO: should be able to play any chan file in any chan system
    for i in num_channels..sys_chan {
        file_data.push(file_data[0].clone());
    }

    let file_data_clone = file_data.clone();

    let mut resampled_data: Vec<Vec<f32>> = vec![vec![]; sys_chan];

    for i in 0..sys_chan {
        let mut source = dasp_signal::from_iter(file_data[i].iter().cloned());
        let a = source.next();
        let b = source.next();
        let interp = Linear::new(a, b);
        let resampled_sig = source
            .from_hz_to_hz(interp, source_sr, sys_sr)
            .until_exhausted();

        resampled_data[i] = resampled_sig.collect();
    }
    let length = resampled_data[0].len();

    let sample_format = config.sample_format();
    let pointer = Arc::new(AtomicUsize::new(0));
    let is_paused = Arc::new(AtomicBool::new(false));

    let err_fn = |err| eprintln!("an error occurred on the output stream: {}", err);

    let is_paused_clone = is_paused.clone();
    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(sys_chan) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < resampled_data.len() && p < length {
                            data[i + j] = resampled_data[j][p];
                        }
                    }

                    if !is_paused_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        let next = if p + 1 < length { p + 1 } else { 0 }; // Loop at the end
                        pointer.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config.into(),
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(sys_chan) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < resampled_data.len() && p < length {
                            data[i + j] = (resampled_data[j][p] * i16::MAX as f32) as i16;
                        }
                    }

                    if !is_paused_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        let next = if p + 1 < length { p + 1 } else { 0 }; // Loop at the end
                        pointer.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config.into(),
            move |data: &mut [u16], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(sys_chan) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < resampled_data.len() && p < length {
                            data[i + j] =
                                ((resampled_data[j][p] * u16::MAX as f32) + u16::MAX as f32) as u16;
                        }
                    }

                    if !is_paused_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        let next = if p + 1 < length { p + 1 } else { 0 }; // Loop at the end
                        pointer.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            err_fn,
            None,
        )?,

        cpal::SampleFormat::I32 => device.build_output_stream(
            &config.into(),
            move |data: &mut [i32], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(sys_chan) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < resampled_data.len() && p < length {
                            data[i + j] = (resampled_data[j][p] * i32::MAX as f32) as i32;
                        }
                    }

                    if !is_paused_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        let next = if p + 1 < length { p + 1 } else { 0 }; // Loop at the end
                        pointer.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            err_fn,
            None,
        )?,
        cpal::SampleFormat::U32 => device.build_output_stream(
            &config.into(),
            move |data: &mut [u32], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(sys_chan) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < resampled_data.len() && p < length {
                            data[i + j] =
                                ((resampled_data[j][p] * u32::MAX as f32) + u32::MAX as f32) as u32;
                        }
                    }

                    if !is_paused_clone.load(std::sync::atomic::Ordering::Relaxed) {
                        let next = if p + 1 < length { p + 1 } else { 0 }; // Loop at the end
                        pointer.store(next, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            },
            err_fn,
            None,
        )?,
        _ => panic!("unsupported sample format"),
    };
    stream.play()?;

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;

    let start_time = Instant::now();
    let file_duration = WavReader::open(file_path)?.duration() as f32 / spec.sample_rate as f32;

    // Initialize angles for canvas animation
    let mut angle1 = 0.0;
    let mut angle2 = 0.0;

    // For tracking playback time
    let mut elapsed = 0.0;
    let mut last_update = Instant::now();

    loop {
        if event::poll(Duration::from_millis(100))? {
            if let event::Event::Key(event) = event::read()? {
                match event.code {
                    KeyCode::Esc => {
                        break;
                    }
                    KeyCode::Char(' ') => {
                        let current_pause_state =
                            is_paused.load(std::sync::atomic::Ordering::Relaxed);
                        is_paused.store(!current_pause_state, std::sync::atomic::Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
        }

        // Update elapsed time only when not paused
        if !is_paused.load(std::sync::atomic::Ordering::Relaxed) {
            let now = Instant::now();
            elapsed += now.duration_since(last_update).as_secs_f32();
            last_update = now;

            // Update angles for rotation only when not paused
            angle1 = (angle1 + 0.1) % (2.0 * PI);
            angle2 = (angle2 + 0.15) % (2.0 * PI);
        } else {
            // Still update last_update to avoid jumps when unpausing
            last_update = Instant::now();
        }

        // Calculate progress based on actual playback progress
        let progress = (elapsed % file_duration) / file_duration;

        // With looping, we'll display the current position within the loop
        let display_time = elapsed % file_duration;

        terminal.draw(|f| {
            let size = f.area();
            let width = size.width as usize;

            // data vec is calculated here, pick width samples from the file data
            let mut data_vec: Vec<(f64, f64)> = vec![];
            for i in 0..width {
                let index = (i as f32 / width as f32 * length as f32) as usize;
                let rms = file_data_clone[0][index];
                data_vec.push((i as f64, rms as f64));
            }

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints(
                    [
                        Constraint::Length(3),      // Progress bar
                        Constraint::Length(8),      // Waveform (reduced height)
                        Constraint::Percentage(70), // Canvas animation
                        Constraint::Min(3),         // Help text
                    ]
                    .as_ref(),
                )
                .split(size);

            let title_text = if is_paused.load(std::sync::atomic::Ordering::Relaxed) {
                format!(
                    "PLAYBACK (PAUSED) {:.2}s/{:.2}s",
                    display_time, file_duration
                )
            } else {
                format!("PLAYBACK {:.2}s/{:.2}s", display_time, file_duration)
            };

            let gauge = Gauge::default()
                .block(Block::default().title(title_text).borders(Borders::ALL))
                .gauge_style(Style::default().fg(Color::Blue).bg(Color::Black))
                .percent((progress * 100.0) as u16);

            f.render_widget(gauge, chunks[0]);

            let datasets = vec![Dataset::default()
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Red))
                .data(&data_vec)];

            let chart = Chart::new(datasets)
                .x_axis(
                    Axis::default()
                        .style(Style::default().fg(Color::Gray))
                        .bounds([0., width as f64]),
                )
                .y_axis(
                    Axis::default()
                        .style(Style::default().fg(Color::Gray))
                        .bounds([-1.0, 1.]),
                )
                .block(Block::default().borders(Borders::ALL).title("Waveform"));

            f.render_widget(chart, chunks[1]);

            // Canvas animation (rotating discs)
            let canvas_rect = chunks[2];

            // Aspect ratio correction
            let canvas_width_chars = canvas_rect.width as f64;
            let canvas_height_chars = canvas_rect.height as f64;
            let char_aspect_ratio = 2.0; // Assume terminal char height is approx 2x width

            // Calculate the aspect ratio needed for world coordinates to make circles appear round
            let canvas_aspect_ratio = canvas_height_chars / canvas_width_chars;
            let world_aspect_ratio = char_aspect_ratio * canvas_aspect_ratio;

            // Define the fixed horizontal range for world coordinates
            let x_world_range = 100.0;
            let x_bounds = [-x_world_range / 2.0, x_world_range / 2.0]; // [-50.0, 50.0]

            // Calculate the corresponding vertical range based on the desired world aspect ratio
            let y_world_range = x_world_range * world_aspect_ratio;
            let y_bounds = [-y_world_range / 2.0, y_world_range / 2.0];

            // Define the circle parameters in world coordinates
            let circle_radius = 15.0;
            let center1_x = -20.0;
            let center1_y = 0.0;
            let center2_x = 20.0;
            let center2_y = 0.0;

            let canvas = Canvas::default()
                .block(Block::default().borders(Borders::ALL).title("Playback"))
                .x_bounds(x_bounds)
                .y_bounds(y_bounds)
                .paint(move |ctx| {
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

                    // Draw rotating lines (left disc)
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

                    // Draw rotating lines (right disc)
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

                    // Draw grooves
                    for r in 1..5 {
                        let groove_radius = circle_radius * (0.3 + r as f64 * 0.15);

                        ctx.draw(&Circle {
                            x: center1_x,
                            y: center1_y,
                            radius: groove_radius,
                            color: Color::Gray,
                        });

                        ctx.draw(&Circle {
                            x: center2_x,
                            y: center2_y,
                            radius: groove_radius,
                            color: Color::Gray,
                        });
                    }
                });

            f.render_widget(canvas, chunks[2]);

            let label = Span::styled(
                "[SPACE] -> PAUSE/RESUME | [ESC] -> QUIT",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::ITALIC | Modifier::BOLD),
            );

            f.render_widget(Paragraph::new(label), chunks[3]);
        })?;
    }

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}
