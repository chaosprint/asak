use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{canvas::Canvas, Block, Borders, List, ListItem, Paragraph, Wrap},
};

use crate::tui::draw_waveform;
use crate::tui::{
    build_dynamic_meter_levels, format_duration, format_sample_rate, render_preview_meter,
    resample_levels, App, AudioPreview, BrowserEntryKind, PlayView, PlaybackSnapshot,
    BRAILLE_PIXELS_PER_CELL_X, BRAILLE_PIXELS_PER_CELL_Y, WAVEFORM_BAR_GAP_DOTS,
    WAVEFORM_BAR_WIDTH_DOTS,
};

pub(crate) fn render_play_tab(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    match &app.play_view {
        PlayView::Browser => render_play_browser_details(area, frame, app),
        PlayView::Preview(preview) => render_play_preview(
            area,
            frame,
            preview,
            app.is_playback_paused(),
            app.playback_snapshot(),
        ),
        PlayView::PreviewError { path, message } => {
            render_play_preview_error(area, frame, path, message)
        }
    }
}

pub(crate) fn render_play_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let items = if app.play_entries.is_empty() {
        vec![ListItem::new(
            "No folders or supported audio files found in the current directory.",
        )]
    } else {
        app.play_entries
            .iter()
            .map(|entry| {
                let label = match entry.kind {
                    BrowserEntryKind::Parent => "../".to_string(),
                    BrowserEntryKind::Directory => format!("{}/", entry.name),
                    BrowserEntryKind::AudioFile => entry.name.clone(),
                };
                ListItem::new(label)
            })
            .collect()
    };

    let mut list_state = app.play_list_state.clone();
    let file_list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Folders"))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    frame.render_stateful_widget(file_list, area, &mut list_state);
}

fn render_play_browser_details(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let preview_lines = if let Some(entry) = app.selected_entry() {
        let selected_kind = match entry.kind {
            BrowserEntryKind::Parent => "Parent directory",
            BrowserEntryKind::Directory => "Directory",
            BrowserEntryKind::AudioFile => "Audio file",
        };

        vec![
            Line::from(vec![
                Span::styled("Current Dir: ", Style::default().fg(Color::Yellow)),
                Span::raw(app.play_dir.display().to_string()),
            ]),
            Line::from(vec![
                Span::styled("Selected: ", Style::default().fg(Color::Yellow)),
                Span::raw(entry.name.clone()),
            ]),
            Line::from(vec![
                Span::styled("Kind: ", Style::default().fg(Color::Yellow)),
                Span::raw(selected_kind),
            ]),
            Line::from(vec![
                Span::styled("Path: ", Style::default().fg(Color::Yellow)),
                Span::raw(entry.path.display().to_string()),
            ]),
            Line::from(""),
            Line::from("Enter opens folders and starts the selected audio preview."),
            Line::from("The left pane stays on folders while the right pane shows details."),
        ]
    } else {
        vec![
            Line::from(vec![
                Span::styled("Current Dir: ", Style::default().fg(Color::Yellow)),
                Span::raw(app.play_dir.display().to_string()),
            ]),
            Line::from(""),
            Line::from("No folders or supported audio files found."),
        ]
    };

    let preview = Paragraph::new(preview_lines)
        .block(Block::default().borders(Borders::ALL).title("Play"))
        .wrap(Wrap { trim: false });
    frame.render_widget(preview, area);
}

fn render_play_preview(
    area: Rect,
    frame: &mut ratatui::Frame<'_>,
    preview: &AudioPreview,
    is_paused: bool,
    playback: Option<PlaybackSnapshot>,
) {
    let playback_position = playback
        .as_ref()
        .map(|snapshot| {
            if snapshot.sample_rate > 0.0 {
                snapshot.pointer as f32 / snapshot.sample_rate as f32
            } else {
                0.0
            }
        })
        .unwrap_or(0.0)
        .min(preview.duration.max(0.0));

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(3),
            Constraint::Length(4),
        ])
        .split(area);

    let header = Paragraph::new(vec![Line::from(vec![
        Span::styled("File: ", Style::default().fg(Color::Yellow)),
        Span::raw(preview.name.clone()),
        Span::raw("   "),
        Span::styled("Format: ", Style::default().fg(Color::Yellow)),
        Span::raw(preview.format.clone()),
        Span::raw("   "),
        Span::styled("Time: ", Style::default().fg(Color::Yellow)),
        Span::raw(format!(
            "{} / {}",
            format_duration(playback_position),
            format_duration(preview.duration)
        )),
        Span::raw("   "),
        Span::styled("Status: ", Style::default().fg(Color::Yellow)),
        Span::raw(if is_paused { "Paused" } else { "Playing" }),
    ])])
    .block(Block::default().borders(Borders::ALL).title("Preview"));
    frame.render_widget(header, layout[0]);

    let waveform_dot_width = layout[1].width as usize * BRAILLE_PIXELS_PER_CELL_X;
    let waveform_dot_height = layout[1].height as usize * BRAILLE_PIXELS_PER_CELL_Y;
    let waveform_bucket_count = (waveform_dot_width + WAVEFORM_BAR_GAP_DOTS)
        / (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS);
    let waveform_levels = resample_levels(&preview.waveform, waveform_bucket_count);
    let playhead_index = playback
        .as_ref()
        .map(|snapshot| {
            if snapshot.frame_len <= 1 || waveform_bucket_count <= 1 {
                0
            } else {
                (((snapshot.pointer.min(snapshot.frame_len.saturating_sub(1))) as f32
                    / snapshot.frame_len.saturating_sub(1) as f32)
                    * waveform_bucket_count.saturating_sub(1) as f32)
                    .round() as usize
            }
        })
        .unwrap_or(0)
        .min(waveform_bucket_count.saturating_sub(1));

    let waveform = Canvas::default()
        .marker(symbols::Marker::Braille)
        .block(Block::default().borders(Borders::ALL).title("Waveform"))
        .x_bounds([0.0, waveform_dot_width.max(1) as f64 - 1.0])
        .y_bounds([0.0, waveform_dot_height.max(1) as f64 - 1.0])
        .paint(move |ctx| {
            draw_waveform(
                ctx,
                &waveform_levels,
                playhead_index,
                waveform_dot_width,
                waveform_dot_height,
            );
        });
    frame.render_widget(waveform, layout[1]);

    let meter_source = playback
        .as_ref()
        .map(build_dynamic_meter_levels)
        .unwrap_or_else(|| preview.meters.clone());
    let meter_block = Block::default().borders(Borders::ALL).title("Meters");
    let meter_inner = meter_block.inner(layout[2]);
    frame.render_widget(meter_block, layout[2]);
    render_preview_meter(meter_inner, frame.buffer_mut(), &meter_source);

    let info = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Yellow)),
            Span::raw(preview.path.display().to_string()),
        ]),
        Line::from(vec![
            Span::styled("Sample Rate: ", Style::default().fg(Color::Yellow)),
            Span::raw(format_sample_rate(preview.sample_rate)),
            Span::raw("   "),
            Span::styled("Channels: ", Style::default().fg(Color::Yellow)),
            Span::raw(preview.channels.to_string()),
        ]),
    ])
    .block(Block::default().borders(Borders::ALL).title("Info"))
    .wrap(Wrap { trim: false });
    frame.render_widget(info, layout[3]);
}

fn render_play_preview_error(
    area: Rect,
    frame: &mut ratatui::Frame<'_>,
    path: &std::path::Path,
    message: &str,
) {
    let error = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Yellow)),
            Span::raw(path.display().to_string()),
        ]),
        Line::from(""),
        Line::from(message.to_string()),
        Line::from(""),
        Line::from("Press Esc or Backspace to return to the browser."),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Preview Error"),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(error, area);
}
