use anyhow::{Context, Result};
use std::{fs::File, path::Path};
use symphonia::core::{
    audio::{AudioBufferRef, Signal},
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error as SymphoniaError,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use symphonia::default::get_probe;

use crate::tui::{build_meter_levels, build_waveform_cache, AudioPreview, WAVEFORM_CACHE_BUCKETS};

pub(crate) fn load_audio_preview(path: &Path) -> Result<AudioPreview> {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .context("Could not determine file extension")?;

    let file = File::open(path)
        .with_context(|| format!("Failed to open audio file '{}'", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    hint.with_extension(&extension);

    let probe_result = get_probe()
        .format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .with_context(|| format!("Failed to probe '{}'", path.display()))?;
    let mut format = probe_result.format;

    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .context("No audio track found")?;

    let sample_rate = track
        .codec_params
        .sample_rate
        .context("Missing sample rate")? as f64;
    let channels = track
        .codec_params
        .channels
        .map(|channels| channels.count())
        .context("Missing channel count")?;
    let duration_hint = track
        .codec_params
        .n_frames
        .map(|frames| frames as f32 / sample_rate as f32);
    let track_id = track.id;

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .context("Failed to create decoder")?;

    let mut samples = vec![Vec::new(); channels];

    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::IoError(ref err))
                if err.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                break;
            }
            Err(err) => return Err(err).context("Failed while reading audio packets"),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let decoded = decoder
            .decode(&packet)
            .context("Failed to decode audio packet")?;
        match decoded {
            AudioBufferRef::F32(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend_from_slice(buf.chan(ch));
                }
            }
            AudioBufferRef::F64(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(buf.chan(ch).iter().map(|&sample| sample as f32));
                }
            }
            AudioBufferRef::S8(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| sample as f32 / i8::MAX as f32),
                    );
                }
            }
            AudioBufferRef::S16(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| sample as f32 / i16::MAX as f32),
                    );
                }
            }
            AudioBufferRef::S24(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|sample| sample.inner() as f32 / 8_388_607.0),
                    );
                }
            }
            AudioBufferRef::S32(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| sample as f32 / i32::MAX as f32),
                    );
                }
            }
            AudioBufferRef::U8(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| (sample as f32 / u8::MAX as f32) * 2.0 - 1.0),
                    );
                }
            }
            AudioBufferRef::U16(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| (sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
                    );
                }
            }
            AudioBufferRef::U24(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|sample| (sample.inner() as f32 / 16_777_215.0) * 2.0 - 1.0),
                    );
                }
            }
            AudioBufferRef::U32(buf) => {
                for (ch, channel_samples) in samples
                    .iter_mut()
                    .enumerate()
                    .take(buf.spec().channels.count().min(channels))
                {
                    channel_samples.extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| (sample as f32 / u32::MAX as f32) * 2.0 - 1.0),
                    );
                }
            }
        }
    }

    let duration = duration_hint.unwrap_or_else(|| {
        samples
            .first()
            .map(|channel| channel.len() as f32 / sample_rate as f32)
            .unwrap_or(0.0)
    });

    let waveform = build_waveform_cache(&samples, WAVEFORM_CACHE_BUCKETS);
    let meters = build_meter_levels(&samples);

    Ok(AudioPreview {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string(),
        path: path.to_path_buf(),
        format: extension.to_ascii_uppercase(),
        duration,
        sample_rate,
        channels,
        samples,
        waveform,
        meters,
    })
}
