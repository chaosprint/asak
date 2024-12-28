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
use ratatui::symbols;
use ratatui::widgets::{Axis, Chart, Dataset, GraphType};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{CrosstermBackend, Terminal, Text},
    style::{Color, Style},
    text::Span,
    widgets::Paragraph,
    widgets::{Block, Borders},
};
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

    let mut shared_waveform_data = Vec::new();

    loop {
        let now = Instant::now();
        let duration = now.duration_since(start_time);
        let recording_time = format!("Recording Time: {:.2}s", duration.as_secs_f32());

        while let Ok(data) = ui_rx.try_recv() {
            shared_waveform_data.extend(data);
        }

        draw_rec_waveform(&mut terminal, &shared_waveform_data, recording_time)?;

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

fn draw_rec_waveform(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    waveform_data: &[f32],
    recording_time: String,
) -> anyhow::Result<()> {
    terminal.draw(|f| {
        let size = f.size();
        let width = size.width as usize;
        let samples_to_use = std::cmp::min(width * 128, waveform_data.len());
        let recent_samples = &waveform_data[waveform_data.len() - samples_to_use..];

        let data_vec: Vec<(f64, f64)> = recent_samples
            .chunks(128)
            .enumerate()
            .map(|(x, samples)| {
                let rms = calculate_rms(samples);
                let x = x as f64;
                (x, rms)
            })
            .collect();

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
                    .bounds([0., 1.]),
            );
        f.render_widget(chart, chunks[1]);
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
    } else {
        if let Some(index) = device {
            host.input_devices()?.nth(index as usize)
        } else {
            panic!("failed to find output device");
        }
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
