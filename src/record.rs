use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SupportedStreamConfig};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use hound::{WavSpec, WavWriter};

use parking_lot::Mutex;
use ratatui::style::Modifier;
use ratatui::symbols;
use ratatui::widgets::{Axis, Chart, Dataset, GraphType};
use std::fs::File;
use std::io::{stdout, BufWriter, Stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use std::time::Duration;

use crossterm::event::{self, KeyCode};

use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{CrosstermBackend, Terminal, Text},
    style::{Color, Style},
    text::Span,
    widgets::Paragraph,
    widgets::{Block, Borders},
};
use std::time::Instant;

fn calculate_rms(samples: &[f32]) -> f64 {
    let square_sum: f64 = samples.iter().map(|&sample| (sample as f64).powi(2)).sum();
    let mean = square_sum / samples.len() as f64;
    mean.sqrt()
}

#[allow(dead_code)]
enum RecordingState {
    AskingInfo,
    Recording,
    Paused,
    Stopped,
}

fn record_tui(
    shared_waveform_data: Arc<RwLock<Vec<f32>>>,
    is_recording: Arc<AtomicBool>,
    recording_state: Arc<Mutex<RecordingState>>,
) -> anyhow::Result<()> {
    let start_time = Instant::now();
    let refresh_interval = Duration::from_millis(100);

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    loop {
        let now = Instant::now();
        let duration = now.duration_since(start_time);
        let recording_time = format!("Recording Time: {:.2}s", duration.as_secs_f32());

        match *recording_state.lock() {
            RecordingState::Recording => {
                draw_rec_waveform(&mut terminal, shared_waveform_data.clone(), recording_time)?;
            }
            _ => {}
        }

        if event::poll(refresh_interval)? {
            if let event::Event::Key(event) = event::read()? {
                if event.code == KeyCode::Enter {
                    is_recording.store(false, Ordering::SeqCst);
                    break;
                } else if event.code == KeyCode::Backspace {
                    // TODO: add pause functionality
                    // } else if event.code == KeyCode::Enter {
                    // start_time = Instant::now();
                    // if let Ok(mut rstate) = recording_state.lock() {
                    //     *rstate = RecordingState::Recording;
                    // }
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
    shared_waveform_data: Arc<RwLock<Vec<f32>>>,
    recording_time: String,
) -> anyhow::Result<()> {
    terminal.draw(|f| {
        let waveform_data = shared_waveform_data.read().unwrap();
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
            format!("press ENTER to exit tui and finish recording..."),
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
                    // .title("X Axis")
                    .style(Style::default().fg(Color::Gray))
                    // .labels(x_labels)
                    .bounds([0., width as f64]),
            )
            .y_axis(
                Axis::default()
                    // .title("RMS")
                    .style(Style::default().fg(Color::Gray))
                    .bounds([0., 1.]),
            );
        f.render_widget(chart, chunks[1]);
    })?;
    Ok(())
}

type WavWriterHandle = Arc<Mutex<Option<hound::WavWriter<BufWriter<File>>>>>;

fn write_input_data<T, U>(input: &[T], writer: &WavWriterHandle)
where
    T: Sample,
    U: Sample + hound::Sample + FromSample<T>,
{
    if let Some(writer) = writer.lock().as_mut() {
        for &sample in input.iter() {
            let sample: U = U::from_sample(sample);
            writer.write_sample(sample).ok();
        }
    }
}

#[allow(unused_variables)]
pub fn record_audio(output: String, device: &str, jack: bool) -> anyhow::Result<()> {
    let output = format!("{}.wav", output.replace(".wav", ""));
    let shared_waveform_data: Arc<RwLock<Vec<f32>>> = Arc::new(RwLock::new(Vec::new()));
    let shared_waveform_data_for_audio_thread = shared_waveform_data.clone();

    let is_recording = Arc::new(AtomicBool::new(true));
    let is_recording_for_thread = is_recording.clone();
    let recording_state = Arc::new(Mutex::new(RecordingState::Recording));

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
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|x| x.name().map(|y| y == device).unwrap_or(false))
    }
    .expect("failed to find output device");

    let config = device.default_input_config().unwrap();

    let o = output.to_owned();
    let recording_thread = std::thread::spawn(move || {
        let path = std::path::Path::new(o.as_str());
        let spec = wav_spec_from_config(&device.default_input_config().unwrap());

        let writer = WavWriter::create(path, spec).unwrap();
        let writer = Arc::new(Mutex::new(Some(writer)));
        // A flag to indicate that recording is in progress.
        // println!("Begin recording...");
        // Run the input stream on a separate thread.
        let writer_2 = writer.clone();
        let err_fn = move |err| eprintln!("an error occurred on stream: {}", err);
        let stream = match config.sample_format() {
            cpal::SampleFormat::I8 => device.build_input_stream(
                &config.into(),
                move |data, _: &_| {
                    if let Ok(mut waveform_data) = shared_waveform_data_for_audio_thread.write() {
                        waveform_data
                            .extend(data.iter().map(|&sample| sample as f32 / i8::MAX as f32));
                    };

                    write_input_data::<i8, i8>(data, &writer_2)
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data, _: &_| {
                    if let Ok(mut waveform_data) = shared_waveform_data_for_audio_thread.write() {
                        waveform_data
                            .extend(data.iter().map(|&sample| sample as f32 / i16::MAX as f32));
                    };

                    write_input_data::<i16, i16>(data, &writer_2)
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I32 => device.build_input_stream(
                &config.into(),
                move |data, _: &_| {
                    if let Ok(mut waveform_data) = shared_waveform_data_for_audio_thread.write() {
                        waveform_data
                            .extend(data.iter().map(|&sample| sample as f32 / i32::MAX as f32));
                    };
                    write_input_data::<i32, i32>(data, &writer_2)
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data, _: &_| {
                    if let Ok(mut waveform_data) = shared_waveform_data_for_audio_thread.write() {
                        waveform_data.extend(data);
                    };
                    write_input_data::<f32, f32>(data, &writer_2)
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
        stream.play().unwrap();

        while is_recording_for_thread.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        stream.pause().unwrap();

        if let Some(writer) = writer.lock().take() {
            writer.finalize().unwrap();
        }

        Ok(())
    });

    record_tui(
        shared_waveform_data,
        is_recording.clone(),
        recording_state.clone(),
    )?;

    is_recording.store(false, Ordering::SeqCst);
    recording_thread.join().unwrap()?;

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
