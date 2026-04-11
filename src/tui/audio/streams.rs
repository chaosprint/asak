use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam::channel::{unbounded, Sender};
use dasp_interpolate::linear::Linear;
use dasp_signal::Signal as DaspSignal;
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Instant;

use crate::tui::{default_recording_name, AudioPreview, PlaybackSession, RecordingSession};

pub(crate) fn start_playback(preview: &AudioPreview) -> Result<PlaybackSession> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .context("No default output device available")?;
    let config = device.default_output_config()?;

    let output_channels = config.channels() as usize;
    let output_sample_rate = config.sample_rate().0 as f64;
    let source_channels = preview.samples.len().max(1);

    let mut resampled_data = vec![Vec::new(); output_channels];
    for output_channel in 0..output_channels {
        let source_index = output_channel.min(source_channels - 1);
        let source_samples = preview.samples[source_index].clone();
        let mut source = dasp_signal::from_iter(source_samples.into_iter());
        let a = DaspSignal::next(&mut source);
        let b = DaspSignal::next(&mut source);
        let interp = Linear::new(a, b);
        let signal =
            DaspSignal::from_hz_to_hz(source, interp, preview.sample_rate, output_sample_rate)
                .until_exhausted();
        resampled_data[output_channel] = signal.collect();
    }

    let frame_len = resampled_data
        .first()
        .map(|channel| channel.len())
        .context("Decoded audio had no samples")?;
    let samples = Arc::new(resampled_data);
    let pointer = Arc::new(AtomicUsize::new(0));
    let is_paused = Arc::new(AtomicBool::new(false));

    let err_fn = |err| eprintln!("audio stream error: {err}");

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(
            &device,
            &config.into(),
            samples.clone(),
            pointer.clone(),
            is_paused.clone(),
            frame_len,
            err_fn,
        )?,
        cpal::SampleFormat::I16 => build_stream::<i16>(
            &device,
            &config.into(),
            samples.clone(),
            pointer.clone(),
            is_paused.clone(),
            frame_len,
            err_fn,
        )?,
        cpal::SampleFormat::U16 => build_stream::<u16>(
            &device,
            &config.into(),
            samples.clone(),
            pointer.clone(),
            is_paused.clone(),
            frame_len,
            err_fn,
        )?,
        cpal::SampleFormat::I32 => build_stream::<i32>(
            &device,
            &config.into(),
            samples.clone(),
            pointer.clone(),
            is_paused.clone(),
            frame_len,
            err_fn,
        )?,
        cpal::SampleFormat::U32 => build_stream::<u32>(
            &device,
            &config.into(),
            samples.clone(),
            pointer.clone(),
            is_paused.clone(),
            frame_len,
            err_fn,
        )?,
        _ => anyhow::bail!("Unsupported output sample format"),
    };

    stream.play()?;

    Ok(PlaybackSession {
        _stream: stream,
        is_paused,
        pointer,
        samples,
        frame_len,
        sample_rate: output_sample_rate,
    })
}

pub(crate) fn start_recording_session(name: &str) -> Result<RecordingSession> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .context("No default input device available")?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| "Unknown input device".to_string());
    let config = device.default_input_config()?;
    let sample_rate = config.sample_rate().0 as f64;
    let channels = config.channels() as usize;

    let output_path = recording_output_path(name)?;
    let (ui_tx, rx) = unbounded();
    let (writer_tx, writer_rx) = unbounded();
    let err_fn = |err| eprintln!("recording stream error: {err}");

    let writer_spec = WavSpec {
        channels: channels as u16,
        sample_rate: config.sample_rate().0,
        bits_per_sample: 32,
        sample_format: WavSampleFormat::Float,
    };

    let writer_path = output_path.clone();
    let writer_thread = std::thread::spawn(move || -> Result<()> {
        let mut writer = WavWriter::create(&writer_path, writer_spec)
            .with_context(|| format!("Failed to create '{}'", writer_path.display()))?;

        while let Ok(chunk) = writer_rx.recv() {
            for sample in chunk {
                writer.write_sample(sample)?;
            }
        }

        writer.finalize()?;
        Ok(())
    });

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_input_stream::<f32, _>(
            &device,
            &config.into(),
            ui_tx.clone(),
            writer_tx.clone(),
            |sample| sample,
            err_fn,
        )?,
        cpal::SampleFormat::I8 => build_input_stream::<i8, _>(
            &device,
            &config.into(),
            ui_tx.clone(),
            writer_tx.clone(),
            |sample| sample as f32 / i8::MAX as f32,
            err_fn,
        )?,
        cpal::SampleFormat::I16 => build_input_stream::<i16, _>(
            &device,
            &config.into(),
            ui_tx.clone(),
            writer_tx.clone(),
            |sample| sample as f32 / i16::MAX as f32,
            err_fn,
        )?,
        cpal::SampleFormat::I32 => build_input_stream::<i32, _>(
            &device,
            &config.into(),
            ui_tx.clone(),
            writer_tx.clone(),
            |sample| sample as f32 / i32::MAX as f32,
            err_fn,
        )?,
        cpal::SampleFormat::U16 => build_input_stream::<u16, _>(
            &device,
            &config.into(),
            ui_tx.clone(),
            writer_tx.clone(),
            |sample| (sample as f32 / u16::MAX as f32) * 2.0 - 1.0,
            err_fn,
        )?,
        cpal::SampleFormat::U32 => build_input_stream::<u32, _>(
            &device,
            &config.into(),
            ui_tx,
            writer_tx,
            |sample| (sample as f32 / u32::MAX as f32) * 2.0 - 1.0,
            err_fn,
        )?,
        _ => anyhow::bail!("Unsupported input sample format"),
    };

    stream.play()?;

    Ok(RecordingSession {
        _stream: stream,
        rx,
        output_path,
        writer_thread,
        started_at: Instant::now(),
        sample_rate,
        channels,
        device_name,
        recent_samples: Vec::new(),
        session_samples: Vec::new(),
    })
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Vec<Vec<f32>>>,
    pointer: Arc<AtomicUsize>,
    is_paused: Arc<AtomicBool>,
    frame_len: usize,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream>
where
    T: cpal::Sample + cpal::SizedSample + cpal::FromSample<f32>,
{
    let channel_count = config.channels as usize;
    Ok(device.build_output_stream(
        config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            for frame in output.chunks_mut(channel_count) {
                let frame_index = pointer.load(Ordering::Relaxed);

                for (channel_index, sample) in frame.iter_mut().enumerate() {
                    let value = samples
                        .get(channel_index)
                        .and_then(|channel| channel.get(frame_index))
                        .copied()
                        .unwrap_or(0.0);
                    *sample = T::from_sample(value);
                }

                if !is_paused.load(Ordering::Relaxed) {
                    let next = if frame_index + 1 < frame_len {
                        frame_index + 1
                    } else {
                        0
                    };
                    pointer.store(next, Ordering::Relaxed);
                }
            }
        },
        err_fn,
        None,
    )?)
}

fn build_input_stream<T, F>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    ui_tx: Sender<Vec<f32>>,
    writer_tx: Sender<Vec<f32>>,
    convert: F,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream>
where
    T: cpal::SizedSample + Copy,
    F: Fn(T) -> f32 + Copy + Send + 'static,
{
    Ok(device.build_input_stream(
        config,
        move |input: &[T], _: &cpal::InputCallbackInfo| {
            let chunk = input.iter().copied().map(convert).collect::<Vec<_>>();
            let _ = ui_tx.send(chunk.clone());
            let _ = writer_tx.send(chunk);
        },
        err_fn,
        None,
    )?)
}

fn recording_output_path(name: &str) -> Result<std::path::PathBuf> {
    let trimmed = name.trim();
    let file_name = if trimmed.is_empty() {
        default_recording_name()
    } else {
        trimmed.to_string()
    };

    let mut path = std::env::current_dir()?.join(file_name);
    path.set_extension("wav");
    Ok(path)
}
