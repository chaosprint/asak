use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    symbols,
    text::{Line, Span},
    widgets::{canvas::Canvas, Block, Borders, Paragraph, Wrap},
};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::tui::{
    build_waveform_cache_mono, char_to_byte_index, draw_waveform, format_duration,
    format_sample_rate, App, BRAILLE_PIXELS_PER_CELL_X, BRAILLE_PIXELS_PER_CELL_Y,
    RECENT_WAVEFORM_VIEW_SECONDS, WAVEFORM_BAR_GAP_DOTS, WAVEFORM_BAR_WIDTH_DOTS,
};

pub(crate) fn render_rec_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let Some(session) = &app.recording_session else {
        let blink_on = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| (duration.as_millis() / 500) % 2 == 0)
            .unwrap_or(true);

        let outer = Block::default()
            .borders(Borders::ALL)
            .title("Rec")
            .border_style(Style::default().fg(Color::Cyan));
        let inner = outer.inner(area);
        frame.render_widget(outer, area);

        let sidebar_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(3),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(inner);

        let heading = Paragraph::new("Enter a file name, then press Enter.")
            .style(Style::default().fg(Color::White));
        frame.render_widget(heading, sidebar_layout[0]);

        let input_block = Block::default()
            .borders(Borders::ALL)
            .title("File Name")
            .border_style(Style::default().fg(Color::LightCyan));
        let input_inner = input_block.inner(sidebar_layout[1]);
        frame.render_widget(input_block, sidebar_layout[1]);

        let cursor_byte = char_to_byte_index(&app.rec_file_name, app.rec_cursor_position);
        let before_cursor = &app.rec_file_name[..cursor_byte];
        let next_char = app.rec_file_name[cursor_byte..].chars().next();
        let next_char_len = next_char.map(|ch| ch.len_utf8()).unwrap_or(0);
        let after_cursor = if next_char_len > 0 {
            &app.rec_file_name[cursor_byte + next_char_len..]
        } else {
            ""
        };
        let cursor_span = match (blink_on, next_char) {
            (true, Some(ch)) => Span::styled(
                ch.to_string(),
                Style::default().bg(Color::LightCyan).fg(Color::Black),
            ),
            (true, None) => {
                Span::styled(" ", Style::default().bg(Color::LightCyan).fg(Color::Black))
            }
            (false, Some(ch)) => Span::styled(ch.to_string(), Style::default().fg(Color::Yellow)),
            (false, None) => Span::raw(" "),
        };
        let input_line = Line::from(vec![
            Span::styled(before_cursor, Style::default().fg(Color::Yellow)),
            cursor_span,
            Span::styled(after_cursor, Style::default().fg(Color::Yellow)),
        ]);
        frame.render_widget(Paragraph::new(input_line), input_inner);

        let mut lines = vec![Line::from(
            "The typed filename is used directly. If the suffix is missing, .wav is added.",
        )];

        if let Some(path) = &app.last_recording_path {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Last saved: ", Style::default().fg(Color::Yellow)),
                Span::raw(path.display().to_string()),
            ]));
        }

        if let Some(message) = &app.recording_error {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("Error: ", Style::default().fg(Color::Red)),
                Span::raw(message.clone()),
            ]));
        }

        let details = Paragraph::new(lines)
            .style(Style::default().fg(Color::Gray))
            .wrap(Wrap { trim: false });
        frame.render_widget(details, sidebar_layout[3]);
        return;
    };

    let status = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("File: ", Style::default().fg(Color::Yellow)),
            Span::raw(session.output_path.display().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Input: ", Style::default().fg(Color::Yellow)),
            Span::raw(session.device_name.clone()),
        ]),
        Line::from(vec![
            Span::styled("Time: ", Style::default().fg(Color::Yellow)),
            Span::raw(format_duration(session.started_at.elapsed().as_secs_f32())),
        ]),
        Line::from(vec![
            Span::styled("Rate: ", Style::default().fg(Color::Yellow)),
            Span::raw(format_sample_rate(session.sample_rate)),
        ]),
        Line::from(vec![
            Span::styled("Channels: ", Style::default().fg(Color::Yellow)),
            Span::raw(session.channels.to_string()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Press Enter to stop.",
            Style::default().fg(Color::Yellow),
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("Rec"))
    .wrap(Wrap { trim: false });
    frame.render_widget(status, area);
}

pub(crate) fn render_rec_main(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let Some(session) = &app.recording_session else {
        let info = Paragraph::new(if app.pending_recording_stop.is_some() {
            "Finalizing recording..."
        } else {
            "Ready to record."
        })
        .block(Block::default().borders(Borders::ALL).title("Rec Overview"))
        .wrap(Wrap { trim: false });
        frame.render_widget(info, area);
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(12), Constraint::Length(6)])
        .split(area);

    let elapsed_label = format_duration(session.started_at.elapsed().as_secs_f32());
    let window_frames =
        ((session.sample_rate * RECENT_WAVEFORM_VIEW_SECONDS).round() as usize).max(1);

    let recent_block = Block::default().borders(Borders::ALL).title("Monitor");
    let recent_inner = recent_block.inner(layout[0]);
    let recent_dot_width = recent_inner.width as usize * BRAILLE_PIXELS_PER_CELL_X;
    let recent_dot_height = recent_inner.height as usize * BRAILLE_PIXELS_PER_CELL_Y;
    let recent_bucket_count = (recent_dot_width + WAVEFORM_BAR_GAP_DOTS)
        / (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS);
    let (recent_levels, recent_playhead) =
        build_recent_monitor_levels(&session.recent_samples, window_frames, recent_bucket_count);

    let recent_waveform = Canvas::default()
        .marker(symbols::Marker::Braille)
        .block(recent_block)
        .x_bounds([0.0, recent_dot_width.max(1) as f64 - 1.0])
        .y_bounds([0.0, recent_dot_height.max(1) as f64 - 1.0])
        .paint(move |ctx| {
            draw_waveform(
                ctx,
                &recent_levels,
                recent_playhead,
                recent_dot_width,
                recent_dot_height,
            );
        });
    frame.render_widget(recent_waveform, layout[0]);

    let session_block = Block::default()
        .borders(Borders::ALL)
        .title(format!("Timeline 00:00 -> {elapsed_label}"));
    let session_inner = session_block.inner(layout[1]);
    let session_dot_width = session_inner.width as usize * BRAILLE_PIXELS_PER_CELL_X;
    let session_dot_height = session_inner.height as usize * BRAILLE_PIXELS_PER_CELL_Y;
    let session_bucket_count = (session_dot_width + WAVEFORM_BAR_GAP_DOTS)
        / (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS);
    let session_levels = build_waveform_cache_mono(&session.session_samples, session_bucket_count);
    let session_playhead = session_bucket_count.saturating_sub(1);

    let overview_waveform = Canvas::default()
        .marker(symbols::Marker::Braille)
        .block(session_block)
        .x_bounds([0.0, session_dot_width.max(1) as f64 - 1.0])
        .y_bounds([0.0, session_dot_height.max(1) as f64 - 1.0])
        .paint(move |ctx| {
            draw_waveform(
                ctx,
                &session_levels,
                session_playhead,
                session_dot_width,
                session_dot_height,
            );
        });
    frame.render_widget(overview_waveform, layout[1]);
}

fn build_recent_monitor_levels(
    samples: &[f32],
    window_frames: usize,
    bucket_count: usize,
) -> (Vec<f32>, usize) {
    if bucket_count == 0 {
        return (Vec::new(), 0);
    }

    let playhead_index = bucket_count / 2;
    if samples.is_empty() {
        return (vec![0.0; bucket_count], playhead_index);
    }

    let history_frames = (window_frames / 2).max(1);
    let history_bucket_count = (playhead_index + 1).max(1);
    let visible_samples = if samples.len() > history_frames {
        &samples[samples.len() - history_frames..]
    } else {
        samples
    };

    let filled_bucket_count = (((visible_samples.len().min(history_frames)) as f32
        / history_frames as f32)
        * history_bucket_count as f32)
        .ceil() as usize;
    let filled_bucket_count = filled_bucket_count.clamp(1, history_bucket_count);

    let active_levels = build_waveform_cache_mono(visible_samples, filled_bucket_count);
    let mut levels = vec![0.0; bucket_count];
    levels[..filled_bucket_count].copy_from_slice(&active_levels);
    (levels, playhead_index)
}
