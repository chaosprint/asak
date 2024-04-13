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

use dasp_interpolate::linear::Linear;
use dasp_signal::Signal;
use std::io::stdout;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[allow(unused_variables)]
pub fn play_audio(file_path: &str, device: &str, jack: bool) -> Result<()> {
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

    let device = if device == "default" {
        host.default_output_device()
    } else {
        host.output_devices()?
            .find(|x| x.name().map(|y| y == device).unwrap_or(false))
    }
    .expect("failed to find output device");

    let config = device.default_output_config().unwrap();

    let sys_chan = config.channels() as usize;
    let sys_sr = config.sample_rate().0 as f64;
    let mut reader = WavReader::open(file_path)?;
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
                    file_data[channel].push(sample as f32);
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
    if sys_chan == 2 && num_channels == 1 {
        file_data.push(file_data[0].clone());
    }

    let file_data_clone = file_data.clone();
    let length = file_data[0].len();

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

    let sample_format = config.sample_format();
    let pointer = Arc::new(AtomicUsize::new(0));

    let err_fn = |err| eprintln!("an error occurred on the output stream: {}", err);

    let stream = match sample_format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config.into(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let channels = sys_chan as usize;
                for i in (0..data.len()).step_by(sys_chan) {
                    let p = pointer.load(std::sync::atomic::Ordering::Relaxed);

                    for j in 0..channels {
                        if i + j < data.len() && j < resampled_data.len() {
                            data[i + j] = resampled_data[j][p];
                        }
                    }

                    let next = if p + 1 < length { p + 1 } else { 0 };
                    pointer.store(next, std::sync::atomic::Ordering::Relaxed);
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
                        if i + j < data.len() && j < resampled_data.len() {
                            data[i + j] = (resampled_data[j][p] * i16::MAX as f32) as i16;
                        }
                    }

                    let next = if p + 1 < length { p + 1 } else { 0 };
                    pointer.store(next, std::sync::atomic::Ordering::Relaxed);
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
                        if i + j < data.len() && j < resampled_data.len() {
                            data[i + j] =
                                ((resampled_data[j][p] * u16::MAX as f32) + u16::MAX as f32) as u16;
                        }
                    }

                    let next = if p + 1 < length { p + 1 } else { 0 };
                    pointer.store(next, std::sync::atomic::Ordering::Relaxed);
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
                        if i + j < data.len() && j < resampled_data.len() {
                            data[i + j] = (resampled_data[j][p] * i32::MAX as f32) as i32;
                        }
                    }

                    let next = if p + 1 < length { p + 1 } else { 0 };
                    pointer.store(next, std::sync::atomic::Ordering::Relaxed);
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
                        if i + j < data.len() && j < resampled_data.len() {
                            data[i + j] =
                                ((resampled_data[j][p] * u32::MAX as f32) + u32::MAX as f32) as u32;
                        }
                    }

                    let next = if p + 1 < length { p + 1 } else { 0 };
                    pointer.store(next, std::sync::atomic::Ordering::Relaxed);
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
