use std::{
    io::{stdout, Stdout},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

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
    prelude::{CrosstermBackend, Terminal},
    style::{Color, Style},
    text::{Span, Text},
    widgets::{BarChart, Paragraph},
};

use ratatui::style::Modifier;

const BUFFER_LENGTH: usize = 4096;

pub fn start_monitoring() -> Result<()> {
    let shared_waveform_data: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
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

    // let audio_thread = std::thread::spawn(move || {
    match stream_format {
        cpal::SampleFormat::F32 => build_stream::<f32>(
            &input_device,
            &config.clone().into(),
            &output_device,
            &config.clone().into(),
            Arc::clone(&is_monitoring),
            shared_waveform_data_for_audio_thread,
        )?,
        cpal::SampleFormat::I16 => build_stream::<i16>(
            &input_device,
            &config.clone().into(),
            &output_device,
            &config.clone().into(),
            Arc::clone(&is_monitoring),
            shared_waveform_data_for_audio_thread,
        )?,
        cpal::SampleFormat::U16 => build_stream::<u16>(
            &input_device,
            &config.clone().into(),
            &output_device,
            &config.clone().into(),
            Arc::clone(&is_monitoring),
            shared_waveform_data_for_audio_thread,
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
    shared_waveform_data: Arc<Mutex<Vec<f32>>>,
) -> Result<(), anyhow::Error>
where
    T: cpal::Sample + Send + 'static + Default + SizedSample + Into<f32>,
{
    let (tx, rx) = bounded::<T>(BUFFER_LENGTH);
    // let is_monitoring_clone = Arc::clone(&is_monitoring);
    let input_stream = input_device.build_input_stream(
        input_config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if is_monitoring.load(Ordering::SeqCst) {
                let mut waveform = shared_waveform_data.lock().unwrap();
                for &sample in data.iter() {
                    let sample_f32: f32 = sample.into();
                    waveform.push(sample_f32);
                    if waveform.len() > BUFFER_LENGTH {
                        waveform.remove(0);
                    }
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
    shared_waveform_data: Arc<Mutex<Vec<f32>>>,
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
    shared_waveform_data: Arc<Mutex<Vec<f32>>>,
    selected_input: &str,
    selected_output: &str,
) -> Result<()> {
    terminal.draw(|f| {
        let waveform_data = shared_waveform_data.lock().unwrap();

        let rms = calculate_rms(&waveform_data);
        let peak = waveform_data.iter().fold(0., |acc, &x| x.abs().max(acc));
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

        let devices = Paragraph::new(Text::raw(format!(
            "INPUT: {};\t  OUTPUT: {};\t MAX VALUE: 100",
            selected_input, selected_output
        )))
        .style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        );
        // f.render_widget(block, chunks[0]);
        f.render_widget(devices, chunks[0]);

        let label = Span::styled(
            "Press ENTER to exit TUI and stop monitoring...",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::ITALIC | Modifier::BOLD),
        );

        f.render_widget(Paragraph::new(label), chunks[2]);

        let data = vec![("RMS", (rms * 100.) as u64), ("PEAK", (peak * 100.) as u64)];

        let chart = BarChart::default()
            // .block(Block::bordered().title("Data1"))
            .data(&data)
            .bar_width(9)
            .bar_gap(6)
            .max(100)
            .bar_style(Style::default().fg(Color::Yellow))
            .value_style(Style::default().fg(Color::Black).bg(Color::Yellow));

        f.render_widget(chart, chunks[1]);
    })?;
    Ok(())
}

fn calculate_rms(samples: &[f32]) -> f64 {
    let square_sum: f64 = samples.iter().map(|&sample| (sample as f64).powi(2)).sum();
    let mean = square_sum / samples.len() as f64;
    mean.sqrt()
}
