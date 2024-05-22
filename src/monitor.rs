use std::{
    io::{stdout, Stdout},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use parking_lot::Mutex;

use anyhow::Result;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SizedSample, SupportedStreamConfig,
};

use crossbeam::channel::bounded;
use inquire::Select;

use crossterm::event::{self, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    prelude::{CrosstermBackend, Terminal, *},
    style::{Color, Style},
    symbols,
    text::{Line, Span, Text},
    widgets::{BarChart, Block, Borders, Gauge, LineGauge, Paragraph},
};

use ratatui::style::Modifier;
use ringbuf::{storage::Heap, traits::*, HeapRb, SharedRb};

pub fn start_monitoring(buffer_length: usize) -> Result<()> {
    let rb = HeapRb::<f32>::new(buffer_length);
    let shared_waveform_data = Arc::new(Mutex::new(rb));
    let shared_waveform_data_for_audio_thread = shared_waveform_data.clone();
    let is_monitoring = Arc::new(AtomicBool::new(true));

    let host = cpal::default_host();
    let devices = host.devices()?;
    let mut device_options = vec![];
    for device in devices {
        device_options.push(device.name().unwrap());
    }
    let selected_input = Select::new("Select an input device:", device_options.clone()).prompt()?;
    let selected_output = Select::new("Select an output device:", device_options).prompt()?;

    let input_device = host
        .devices()?
        .find(|device| device.name().unwrap() == selected_input)
        .unwrap();
    let output_device = host
        .devices()?
        .find(|device| device.name().unwrap() == selected_output)
        .unwrap();

    // todo: selected_output has to be the default output device manually, which is a bug
    // let output_device = host.default_output_device().unwrap();
    // let selected_output = output_device.name().unwrap();

    let input_config = input_device.default_input_config()?;
    let output_config = output_device.default_output_config()?;

    if input_config.sample_rate() != output_config.sample_rate() {
        return Err(anyhow::anyhow!(
            "Sample rates of input and output devices do not match."
        ));
    }

    let config = SupportedStreamConfig::new(
        2,
        input_config.sample_rate(),
        input_config.buffer_size().clone(),
        input_config.sample_format(),
    );

    let stream_format = input_config.sample_format();

    match stream_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(
            &input_device,
            &config.clone().into(),
            &output_device,
            &config.clone().into(),
            Arc::clone(&is_monitoring),
            shared_waveform_data_for_audio_thread,
            buffer_length,
        )?,
        cpal::SampleFormat::I16 => build_stream::<i16>(
            &input_device,
            &config.clone().into(),
            &output_device,
            &config.clone().into(),
            Arc::clone(&is_monitoring),
            shared_waveform_data_for_audio_thread,
            buffer_length,
        )?,
        cpal::SampleFormat::U16 => build_stream::<u16>(
            &input_device,
            &config.clone().into(),
            &output_device,
            &config.clone().into(),
            Arc::clone(&is_monitoring),
            shared_waveform_data_for_audio_thread,
            buffer_length,
        )?,
        _ => return Err(anyhow::anyhow!("Unsupported sample format")),
    };

    record_tui(
        shared_waveform_data,
        is_monitoring,
        &selected_input,
        &selected_output,
    )?;
    Ok(())
}

fn build_stream<T>(
    input_device: &cpal::Device,
    input_config: &cpal::StreamConfig,
    output_device: &cpal::Device,
    output_config: &cpal::StreamConfig,
    is_monitoring: Arc<AtomicBool>,
    shared_waveform_data: Arc<Mutex<SharedRb<Heap<f32>>>>,
    buffer_length: usize,
) -> Result<(), anyhow::Error>
where
    T: cpal::Sample + Send + 'static + Default + SizedSample + Into<f32>,
{
    let (tx, rx) = bounded::<T>(buffer_length);
    // let is_monitoring_clone = Arc::clone(&is_monitoring);
    let input_stream = input_device.build_input_stream(
        input_config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if is_monitoring.load(Ordering::SeqCst) {
                let mut waveform = shared_waveform_data.lock();
                for &sample in data.iter() {
                    let sample_f32: f32 = sample.into();
                    (*waveform).push_overwrite(sample_f32);
                    if tx.send(sample).is_err() {
                        eprintln!("Buffer overflow, dropping sample");
                    }
                }
            }
        },
        move |err| {
            eprintln!("Error occurred on input stream: {}", err);
        },
        None,
    )?;

    let output_stream = output_device.build_output_stream(
        output_config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            for sample in data.iter_mut() {
                *sample = rx.recv().unwrap_or_default();
            }
        },
        move |err| {
            eprintln!("Error occurred on output stream: {}", err);
        },
        None,
    )?;

    input_stream.play()?;
    output_stream.play()?;

    Ok(())
}

fn record_tui(
    shared_waveform_data: Arc<Mutex<SharedRb<Heap<f32>>>>,
    is_monitoring: Arc<AtomicBool>,
    selected_input: &str,
    selected_output: &str,
) -> Result<()> {
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;
    terminal.clear()?;

    loop {
        draw_rec_waveform(
            &mut terminal,
            shared_waveform_data.clone(),
            selected_input,
            selected_output,
        )?;
        let refresh_interval = Duration::from_millis(100);
        if event::poll(refresh_interval)? {
            if let event::Event::Key(event) = event::read()? {
                if event.code == KeyCode::Enter {
                    is_monitoring.store(false, Ordering::SeqCst);
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
    shared_waveform_data: Arc<Mutex<SharedRb<Heap<f32>>>>,
    selected_input: &str,
    selected_output: &str,
) -> Result<()> {
    terminal.draw(|f| {
        let waveform_data = shared_waveform_data.lock();
        let waveform: Vec<f32> = waveform_data.iter().copied().collect();

        let size = f.size();

        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Length(2),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    // Constraint::Length(4),
                    // Constraint::Length(4),
                    Constraint::Length(4),
                    Constraint::Length(4),
                    Constraint::Min(4),
                ]
                .as_ref(),
            );

        let [title, indicator, _padding, rect_left, rect_right, help] = vertical.areas(f.size());

        let devices = Paragraph::new(Text::raw(format!(
            "INPUT: {};\t  OUTPUT: {};",
            selected_input, selected_output
        )))
        .style(
            Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
        );
        f.render_widget(devices, title);

        let label = Span::styled(
            "Press ENTER to exit TUI and stop monitoring...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        );

        f.render_widget(Paragraph::new(label), help);

        let level = calculate_level(&waveform);

        if level.len() < 2 {
            return;
        }
        let left = (level[0].0 * 90.) as u64;
        let right = (level[1].0 * 90.) as u64;

        let db_left = (20. * level[0].0.log10()) as i32;
        let db_right = (20. * level[1].0.log10()) as i32;
        // audio clip indicator
        let color = if left > 90 || right > 90 {
            Color::Red
        } else {
            Color::Green
        };

        // let peak_db_left =  (20. * level[1].0.log10()) as i32;
        // let peak_db_right = (20. * level[1].0.log10()) as i32;

        // // render peak left as gauge
        // let g = Gauge::default()
        //     .block(Block::new().title("Left Peak").borders(Borders::ALL))
        //     .gauge_style(color)
        //     .label(Span::styled(
        //         format!(
        //             "{} db",
        //             match peak_db_left {
        //                 x if x < -90 => "-inf".to_string(),
        //                 x => x.to_string(),
        //             }
        //         ),
        //         Style::new().italic().bold().fg(Color::White),
        //     ))
        //     .ratio(level[0].1 as f64 * 0.9);
        // f.render_widget(g, rect_peak_left);

        // // render peak right as gauge
        // let g = Gauge::default()
        //     .block(Block::new().title("Right Peak").borders(Borders::ALL))
        //     .gauge_style(color)
        //     .label(Span::styled(
        //         format!(
        //             "{} db",
        //             match peak_db_right {
        //                 x if x < -90 => "-inf".to_string(),
        //                 x => x.to_string(),
        //             }
        //         ),
        //         Style::new().italic().bold().fg(Color::White),
        //     ))
        //     .ratio(level[1].1 as f64 * 0.9);
        // f.render_widget(g, rect_peak_right);

        let g = Gauge::default()
            .block(Block::new().title("Left dB SPL").borders(Borders::ALL))
            .gauge_style(color)
            .label(Span::styled(
                format!(
                    "{} db",
                    match db_left {
                        x if x < -90 => "-inf".to_string(),
                        x => x.to_string(),
                    }
                ),
                Style::new().italic().bold().fg(Color::White),
            ))
            .ratio(level[0].0 as f64);
        f.render_widget(g, rect_left);

        let g = Gauge::default()
            .block(Block::new().title("Right dB SPL").borders(Borders::ALL))
            .gauge_style(color)
            .label(Span::styled(
                format!(
                    "{} db",
                    match db_right {
                        x if x < -90 => "-inf".to_string(),
                        x => x.to_string(),
                    }
                ),
                Style::new().italic().bold().fg(Color::White),
            ))
            .ratio(level[1].0 as f64);
        f.render_widget(g, rect_right);

        // let peak_left = (level[0].1 * 90.) as u64;
        let [low, high] =
            Layout::horizontal([Constraint::Percentage(90), Constraint::Percentage(10)])
                .areas(indicator);

        // let red_line = Block::default()
        //     .borders(Borders::NONE)
        //     .style(Style::default().bg(Color::Red));

        // f.render_widget(red_line, clippy_indicator);

        let low_level_rect = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(Color::Green));
        f.render_widget(low_level_rect, low);
        let high_level_rect = Block::default()
            .borders(Borders::NONE)
            .style(Style::default().bg(Color::Red));
        f.render_widget(high_level_rect, high);
    })?;
    Ok(())
}

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
        v.push((rms, peak.unwrap()));
    }
    v
}
