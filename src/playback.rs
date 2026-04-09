use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossterm::event::{self, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
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
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_probe;

use dasp_interpolate::linear::Linear;
use dasp_signal::Signal as DaspSignal;
use std::f64::consts::PI;
use std::fs::File;
use std::io::stdout;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Audio file data containing samples, metadata, and format information.
#[derive(Debug, Clone)]
pub struct AudioData {
    /// Audio samples organized by channel. Each inner vector contains samples for one channel.
    /// Samples are normalized to the range [-1.0, 1.0].
    pub samples: Vec<Vec<f32>>,
    /// Sample rate of the audio in Hz.
    pub sample_rate: f64,
    /// Number of audio channels.
    pub channels: usize,
    /// Duration of the audio file in seconds.
    pub duration: f32,
}

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

    let config = device.default_output_config()?;

    let sys_chan = config.channels() as usize;
    let sys_sr = config.sample_rate().0 as f64;

    let audio_data = read_file_data(file_path)?;
    let mut file_data = audio_data.samples;
    let source_sr = audio_data.sample_rate;
    let num_channels = audio_data.channels;

    // TODO: should be able to play any chan file in any chan system
    for i in num_channels..sys_chan {
        file_data.push(file_data[0].clone());
    }

    let file_data_clone = file_data.clone();

    let mut resampled_data: Vec<Vec<f32>> = vec![vec![]; sys_chan];

    for i in 0..sys_chan {
        let mut source = dasp_signal::from_iter(file_data[i].iter().cloned());
        let a = DaspSignal::next(&mut source);
        let b = DaspSignal::next(&mut source);
        let interp = Linear::new(a, b);
        let resampled_sig =
            DaspSignal::from_hz_to_hz(source, interp, source_sr, sys_sr).until_exhausted();

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
                let channels = sys_chan;
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
                let channels = sys_chan;
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
                let channels = sys_chan;
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
                let channels = sys_chan;
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
                let channels = sys_chan;
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
    // Use duration calculated in read_file_data function
    let file_duration = audio_data.duration;

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

/// Loads and decodes audio data from WAV, OGG, or MP3 files for playback.
///
/// ### Arguments
/// * `file_path` - Path to the audio file (must be .wav, .ogg, or .mp3)
///
/// ### Returns
/// A `Result` containing `AudioData` with normalized samples, sample rate, and channel count.
///
/// ### Supported Formats
/// - WAV (PCM and IEEE float)
/// - OGG Vorbis
/// - MP3
fn read_file_data(file_path: &str) -> Result<AudioData> {
    let extension = std::path::Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_lowercase())
        .ok_or_else(|| anyhow::anyhow!("Could not determine file extension"))?;

    // Validate supported formats
    match extension.as_str() {
        "wav" | "ogg" | "mp3" => {}
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported format: {}. Only WAV, OGG, and MP3 are supported.",
                extension
            ))
        }
    }

    let file = File::open(file_path)?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(&extension);

    let meta_opts = MetadataOptions::default();
    let fmt_opts = FormatOptions::default();

    let probe_result = get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| anyhow::anyhow!("Failed to probe format: {}", e))?;
    let mut format = probe_result.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| anyhow::anyhow!("No audio track found in {}", file_path))?;

    let track_id = track.id;
    let sample_rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| anyhow::anyhow!("Sample rate not found"))? as f64;
    let channels = track
        .codec_params
        .channels
        .map(|c| c.count())
        .ok_or_else(|| anyhow::anyhow!("Channel count not found"))?;

    // Calculate duration from track parameters if available
    // Note: this duration calculation must come before the `format.next_packet()` loop to avoid
    // error[E0502]: cannot borrow `*format` as mutable because it is also borrowed as immutable
    let duration_from_frame_count = track
        .codec_params
        .n_frames
        .map(|frames| frames as f32 / sample_rate as f32);

    let dec_opts = DecoderOptions::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| anyhow::anyhow!("Failed to create decoder: {}", e))?;

    let mut audio_samples: Vec<Vec<f32>> = vec![Vec::new(); channels];

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(ref e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(e) => return Err(anyhow::anyhow!("Error reading packet: {}", e)),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let audio_buf = decoder
            .decode(&packet)
            .map_err(|e| anyhow::anyhow!("Decode error: {}", e))?;

        // Convert to f32 samples based on the buffer type
        match audio_buf {
            AudioBufferRef::F32(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch].extend_from_slice(buf.chan(ch));
                }
            }
            AudioBufferRef::F64(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch].extend(buf.chan(ch).iter().map(|&s| s as f32));
                }
            }
            AudioBufferRef::S8(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch]
                        .extend(buf.chan(ch).iter().map(|&s| s as f32 / i8::MAX as f32));
                }
            }
            AudioBufferRef::S16(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch]
                        .extend(buf.chan(ch).iter().map(|&s| s as f32 / i16::MAX as f32));
                }
            }
            AudioBufferRef::S24(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch].extend(
                        buf.chan(ch).iter().map(|&s| s.inner() as f32 / 8388608.0), // 2^23
                    );
                }
            }
            AudioBufferRef::S32(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch]
                        .extend(buf.chan(ch).iter().map(|&s| s as f32 / i32::MAX as f32));
                }
            }
            AudioBufferRef::U8(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch]
                        .extend(buf.chan(ch).iter().map(|&s| (s as f32 - 128.0) / 128.0));
                }
            }
            AudioBufferRef::U16(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch]
                        .extend(buf.chan(ch).iter().map(|&s| (s as f32 - 32768.0) / 32768.0));
                }
            }
            AudioBufferRef::U24(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&s| (s.inner() as f32 - 8388608.0) / 8388608.0),
                    );
                }
            }
            AudioBufferRef::U32(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    audio_samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&s| (s as f32 - 2147483648.0) / 2147483648.0),
                    );
                }
            }
        }
    }

    // Verify we got some audio data
    if audio_samples.is_empty() || audio_samples[0].is_empty() {
        return Err(anyhow::anyhow!("No audio data found in file"));
    }

    // Use pre-calculated duration from track params, or fallback to sample count
    let duration = duration_from_frame_count
        .unwrap_or_else(|| audio_samples[0].len() as f32 / sample_rate as f32);

    Ok(AudioData {
        samples: audio_samples,
        sample_rate,
        channels,
        duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::PathBuf;

    #[rstest]
    fn test_read_file_data(#[files("tests/data/xkeril_melody.*")] path: PathBuf) {
        let result = read_file_data(&path.to_string_lossy());
        assert!(
            result.is_ok(),
            "Failed to load test file {:?}: {:?}",
            path,
            result.err()
        );

        let audio_data = result.unwrap();

        assert!(
            !audio_data.samples.is_empty(),
            "File data should not be empty"
        );
        assert_eq!(
            audio_data.samples.len(),
            audio_data.channels,
            "Number of channels should match data structure"
        );
        assert!(
            audio_data.sample_rate > 0.0,
            "Sample rate should be positive"
        );
        assert!(audio_data.channels > 0, "Should have at least one channel");

        for channel_data in &audio_data.samples {
            assert!(
                !channel_data.is_empty(),
                "Each channel should have sample data"
            );

            for &sample in channel_data {
                assert!(
                    sample >= -1.0 && sample <= 1.0,
                    "Samples should be normalized between -1.0 and 1.0"
                );
            }
        }

        // Assert duration is roughly 10 seconds (±0.5 second error margin)
        assert!(
            (audio_data.duration - 10.0).abs() < 0.5,
            "Duration should be roughly 10 seconds, got {:.2} seconds",
            audio_data.duration
        );

        let format = path.extension().unwrap().to_string_lossy().to_uppercase();
        println!("Test {} file loaded successfully:", format);
        println!("  Sample rate: {} Hz", audio_data.sample_rate);
        println!("  Channels: {}", audio_data.channels);
        println!("  Samples per channel: {}", audio_data.samples[0].len());
        println!("  Duration: {:.2} seconds", audio_data.duration);
    }
}
