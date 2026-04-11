use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Buffer, Widget},
    style::{Color, Style},
    text::{Line, Span},
    widgets::canvas::{Context, Line as CanvasLine},
};

use crate::tui::{
    MeterLevel, PlaybackSnapshot, METER_WINDOW_MS, WAVEFORM_BAR_GAP_DOTS, WAVEFORM_BAR_WIDTH_DOTS,
};

pub(crate) fn build_waveform_cache(channels: &[Vec<f32>], bucket_count: usize) -> Vec<f32> {
    if bucket_count == 0 {
        return Vec::new();
    }

    let active_channels = channels
        .iter()
        .filter(|channel| !channel.is_empty())
        .collect::<Vec<_>>();
    let Some(sample_len) = active_channels.iter().map(|channel| channel.len()).min() else {
        return vec![0.0; bucket_count];
    };

    let mut buckets = Vec::with_capacity(bucket_count);
    for bucket in 0..bucket_count {
        let start = (bucket * sample_len) / bucket_count;
        let mut end = ((bucket + 1) * sample_len) / bucket_count;
        if end <= start {
            end = (start + 1).min(sample_len);
        }

        let mut square_sum = 0.0f64;
        let mut sample_count = 0usize;
        let mut peak = 0.0f32;

        for channel in &active_channels {
            for &sample in &channel[start..end] {
                square_sum += (sample as f64) * (sample as f64);
                peak = peak.max(sample.abs());
                sample_count += 1;
            }
        }

        let rms = if sample_count > 0 {
            (square_sum / sample_count as f64).sqrt() as f32
        } else {
            0.0
        };
        buckets.push((rms * 0.84) + (peak * 0.16));
    }

    normalize_and_smooth(&buckets)
}

pub(crate) fn build_waveform_cache_mono(samples: &[f32], bucket_count: usize) -> Vec<f32> {
    if bucket_count == 0 {
        return Vec::new();
    }

    if samples.is_empty() {
        return vec![0.0; bucket_count];
    }

    let sample_len = samples.len();
    let mut buckets = Vec::with_capacity(bucket_count);

    for bucket in 0..bucket_count {
        let start = (bucket * sample_len) / bucket_count;
        let mut end = ((bucket + 1) * sample_len) / bucket_count;
        if end <= start {
            end = (start + 1).min(sample_len);
        }

        let mut square_sum = 0.0f64;
        let mut sample_count = 0usize;
        let mut peak = 0.0f32;

        for &sample in &samples[start..end] {
            square_sum += (sample as f64) * (sample as f64);
            peak = peak.max(sample.abs());
            sample_count += 1;
        }

        let rms = if sample_count > 0 {
            (square_sum / sample_count as f64).sqrt() as f32
        } else {
            0.0
        };
        buckets.push((rms * 0.84) + (peak * 0.16));
    }

    normalize_and_smooth(&buckets)
}

pub(crate) fn downmix_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 || samples.is_empty() {
        return Vec::new();
    }

    samples
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect()
}

pub(crate) fn build_meter_levels(channels: &[Vec<f32>]) -> Vec<MeterLevel> {
    let mut levels = channels
        .iter()
        .take(2)
        .enumerate()
        .filter(|(_, channel)| !channel.is_empty())
        .map(|(index, channel)| {
            let peak = channel
                .iter()
                .fold(0.0f32, |acc, sample| acc.max(sample.abs()));

            MeterLevel {
                label: if index == 0 { "L" } else { "R" }.to_string(),
                peak,
            }
        })
        .collect::<Vec<_>>();

    if levels.len() == 1 {
        levels[0].label = "Mono".to_string();
    }

    levels
}

fn normalize_and_smooth(levels: &[f32]) -> Vec<f32> {
    let max_level = levels.iter().copied().fold(0.0f32, f32::max);
    let normalized = if max_level > f32::EPSILON {
        levels
            .iter()
            .map(|level| level / max_level)
            .collect::<Vec<_>>()
    } else {
        levels.to_vec()
    };

    if normalized.len() < 3 {
        return normalized;
    }

    let mut smoothed = Vec::with_capacity(normalized.len());
    for index in 0..normalized.len() {
        let left = normalized[index.saturating_sub(1)];
        let center = normalized[index];
        let right = normalized[(index + 1).min(normalized.len() - 1)];
        smoothed.push((left * 0.22) + (center * 0.56) + (right * 0.22));
    }
    smoothed
}

pub(crate) fn resample_levels(levels: &[f32], bucket_count: usize) -> Vec<f32> {
    if bucket_count == 0 || levels.is_empty() {
        return Vec::new();
    }

    (0..bucket_count)
        .map(|index| {
            let source_index = ((index * levels.len()) / bucket_count).min(levels.len() - 1);
            levels[source_index]
        })
        .collect()
}

pub(crate) fn build_dynamic_meter_levels(snapshot: &PlaybackSnapshot) -> Vec<MeterLevel> {
    let window_frames = ((snapshot.sample_rate * METER_WINDOW_MS as f64) / 1000.0)
        .round()
        .max(64.0) as usize;
    let end = snapshot.pointer.min(snapshot.frame_len);
    let start = end.saturating_sub(window_frames);

    snapshot
        .samples
        .iter()
        .take(2)
        .enumerate()
        .filter_map(|(index, channel)| {
            if channel.is_empty() {
                return None;
            }

            let channel_start = start.min(channel.len().saturating_sub(1));
            let channel_end = end.min(channel.len()).max(channel_start + 1);
            let slice = &channel[channel_start..channel_end];

            let peak = slice
                .iter()
                .fold(0.0f32, |acc, sample| acc.max(sample.abs()));

            Some(MeterLevel {
                label: if snapshot.samples.len() == 1 {
                    "Mono".to_string()
                } else if index == 0 {
                    "L".to_string()
                } else {
                    "R".to_string()
                },
                peak,
            })
        })
        .collect()
}

pub(crate) fn draw_waveform(
    ctx: &mut Context<'_>,
    levels: &[f32],
    playhead_index: usize,
    dot_width: usize,
    dot_height: usize,
) {
    if levels.is_empty() || dot_width == 0 || dot_height == 0 {
        return;
    }

    let midpoint = (dot_height as f64 - 1.0) / 2.0;
    let max_bar_radius = midpoint.max(1.0);
    let playhead_x = ((playhead_index * (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS))
        .min(dot_width.saturating_sub(1))) as f64;

    ctx.draw(&CanvasLine {
        x1: playhead_x,
        y1: 0.0,
        x2: playhead_x,
        y2: dot_height as f64 - 1.0,
        color: Color::LightCyan,
    });

    for (index, level) in levels.iter().copied().enumerate() {
        let column_start = index * (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS);
        if column_start >= dot_width {
            break;
        }

        let shaped_level = level.clamp(0.0, 1.0).powf(0.72) as f64;
        let bar_radius = if shaped_level <= 0.0 {
            0.0
        } else {
            (shaped_level * max_bar_radius).max(1.0)
        };
        let top = (midpoint - bar_radius).max(0.0);
        let bottom = (midpoint + bar_radius).min(dot_height as f64 - 1.0);

        for x in column_start..(column_start + WAVEFORM_BAR_WIDTH_DOTS).min(dot_width) {
            ctx.draw(&CanvasLine {
                x1: x as f64,
                y1: top,
                x2: x as f64,
                y2: bottom,
                color: Color::White,
            });
        }
    }
}

fn meter_segment_sizes(peak: f32, width: usize) -> (usize, usize, usize) {
    let db = 20.0 * (peak + 1e-10_f32).log10();
    let vu = db.clamp(-60.0, 6.0);
    let normalize = |value: f32| {
        let amplitude = 10.0_f32.powf(value / 60.0);
        let min = 10.0_f32.powf(-60.0 / 60.0);
        let max = 10.0_f32.powf(6.0 / 60.0);
        (amplitude - min) / (max - min)
    };

    let lit = ((normalize(vu) * width as f32).round() as usize).min(width);
    let zero_segment = ((normalize(0.0) * width as f32).round() as usize).min(width);
    let active = lit.min(zero_segment);
    let overload = lit.saturating_sub(zero_segment);
    let inactive = width.saturating_sub(active).saturating_sub(overload);
    (active, overload, inactive)
}

pub(crate) fn render_preview_meter(area: Rect, buf: &mut Buffer, meters: &[MeterLevel]) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    if meters.is_empty() {
        Line::from("No channel meter data available.")
            .alignment(Alignment::Center)
            .render(area, buf);
        return;
    }

    if meters.len() == 1 {
        let layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(1), Constraint::Fill(2)])
            .spacing(1)
            .split(area);
        let live_area = layout[0];
        let meter_area = layout[1];

        let meter = &meters[0];
        let (active, overload, inactive) =
            meter_segment_sizes(meter.peak, meter_area.width as usize);

        Line::from(vec![Span::styled(
            "▮",
            Style::default().fg(Color::LightGreen),
        )])
        .render(live_area, buf);

        Line::from(vec![
            Span::styled("▮".repeat(active), Style::default().fg(Color::LightGreen)),
            Span::styled("▮".repeat(overload), Style::default().fg(Color::Red)),
            Span::styled("▮".repeat(inactive), Style::default().fg(Color::DarkGray)),
        ])
        .render(meter_area, buf);
        return;
    }

    let left = meters
        .iter()
        .find(|meter| meter.label == "L")
        .cloned()
        .unwrap_or_else(|| meters[0].clone());
    let right = meters
        .iter()
        .find(|meter| meter.label == "R")
        .cloned()
        .unwrap_or_else(|| meters.get(1).cloned().unwrap_or_else(|| meters[0].clone()));

    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(2),
            Constraint::Length(2),
            Constraint::Fill(2),
        ])
        .spacing(1)
        .split(area);

    let left_area = layout[0];
    let center_area = layout[1];
    let right_area = layout[2];

    let (left_active, left_overload, left_inactive) =
        meter_segment_sizes(left.peak, left_area.width as usize);
    let (right_active, right_overload, right_inactive) =
        meter_segment_sizes(right.peak, right_area.width as usize);

    Line::from(vec![
        Span::styled(
            "▮".repeat(left_inactive),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("▮".repeat(left_overload), Style::default().fg(Color::Red)),
        Span::styled(
            "▮".repeat(left_active),
            Style::default().fg(Color::LightGreen),
        ),
    ])
    .alignment(Alignment::Right)
    .render(left_area, buf);

    Line::from(Span::styled("▮▮", Style::default().fg(Color::LightGreen))).render(center_area, buf);

    Line::from(vec![
        Span::styled(
            "▮".repeat(right_active),
            Style::default().fg(Color::LightGreen),
        ),
        Span::styled("▮".repeat(right_overload), Style::default().fg(Color::Red)),
        Span::styled(
            "▮".repeat(right_inactive),
            Style::default().fg(Color::DarkGray),
        ),
    ])
    .render(right_area, buf);
}
