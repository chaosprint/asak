use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::tui::{
    render, ActiveTab, App, NavigationLevel, SettingsField, SettingsLevel, MODE_TITLES,
};

pub(crate) fn render_sidebar(
    area: ratatui::layout::Rect,
    frame: &mut ratatui::Frame<'_>,
    app: &App,
) {
    match app.navigation_level {
        NavigationLevel::TabSelect => render_mode_sidebar(area, frame, app),
        NavigationLevel::TabContent => match app.active_tab {
            ActiveTab::Play => render::render_play_sidebar(area, frame, app),
            ActiveTab::Rec => render::render_rec_sidebar(area, frame, app),
            ActiveTab::Settings => render_settings_sidebar(area, frame, app),
        },
    }
}

pub(crate) fn render_main_panel(
    area: ratatui::layout::Rect,
    frame: &mut ratatui::Frame<'_>,
    app: &App,
) {
    match app.navigation_level {
        NavigationLevel::TabSelect => render_mode_description(area, frame, app),
        NavigationLevel::TabContent => match app.active_tab {
            ActiveTab::Play => render::render_play_tab(area, frame, app),
            ActiveTab::Rec => render::render_rec_main(area, frame, app),
            ActiveTab::Settings => render_settings_main(area, frame, app),
        },
    }
}

pub(crate) fn render_mode_sidebar(
    area: ratatui::layout::Rect,
    frame: &mut ratatui::Frame<'_>,
    app: &App,
) {
    let items = MODE_TITLES
        .iter()
        .map(|title| ListItem::new(*title))
        .collect::<Vec<_>>();
    let mut state = ListState::default();
    state.select(Some(app.active_tab.index()));

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("asak"))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    frame.render_stateful_widget(list, area, &mut state);
}

pub(crate) fn render_mode_description(
    area: ratatui::layout::Rect,
    frame: &mut ratatui::Frame<'_>,
    app: &App,
) {
    let (title, lines) = match app.active_tab {
        ActiveTab::Play => (
            "Play",
            vec![
                Line::from("Browse folders and supported audio files from the left column."),
                Line::from("Open a track to inspect waveform, stereo meter, and playback time."),
                Line::from(""),
                Line::from(Span::styled(
                    "Enter opens the browser in the left pane.",
                    Style::default().fg(Color::Yellow),
                )),
            ],
        ),
        ActiveTab::Rec => (
            "Rec",
            if let Some(path) = &app.last_recording_path {
                vec![
                    Line::from(vec![
                        Span::styled("Saved: ", Style::default().fg(Color::Yellow)),
                        Span::raw(path.display().to_string()),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Select Play to open the new wav.",
                        Style::default().fg(Color::Yellow),
                    )),
                ]
            } else {
                vec![
                    Line::from("Type a filename in the left column before recording starts."),
                    Line::from(
                        "The right side shows the rolling waveform and full-session overview.",
                    ),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Enter opens the recorder input box in the left pane.",
                        Style::default().fg(Color::Yellow),
                    )),
                ]
            },
        ),
        ActiveTab::Settings => (
            "Settings",
            vec![
                Line::from("A placeholder for device and UI preferences."),
                Line::from("This mode is ready for settings content next."),
                Line::from(""),
                Line::from(Span::styled(
                    "Enter opens the settings panel.",
                    Style::default().fg(Color::Yellow),
                )),
            ],
        ),
    };

    let panel = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

pub(crate) fn render_settings_sidebar(
    area: ratatui::layout::Rect,
    frame: &mut ratatui::Frame<'_>,
    app: &App,
) {
    let items = vec![
        ListItem::new(vec![
            Line::from(Span::styled(
                "Playback Device",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                app.device_settings
                    .selected_output_name()
                    .unwrap_or("No output device"),
                Style::default().fg(Color::Gray),
            )),
        ]),
        ListItem::new(vec![
            Line::from(Span::styled(
                "Recording Device",
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                app.device_settings
                    .selected_input_name()
                    .unwrap_or("No input device"),
                Style::default().fg(Color::Gray),
            )),
        ]),
    ];
    let mut state = ListState::default();
    state.select(Some(app.device_settings.focused_field.index()));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(match app.device_settings.level {
                    SettingsLevel::FieldSelect => "Settings",
                    SettingsLevel::DeviceSelect => "Settings > Field",
                }),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    frame.render_stateful_widget(list, area, &mut state);
}

pub(crate) fn render_settings_main(
    area: ratatui::layout::Rect,
    frame: &mut ratatui::Frame<'_>,
    app: &App,
) {
    if !app.device_settings.is_selecting_device() {
        let (title, current, description) = match app.device_settings.focused_field {
            SettingsField::PlaybackDevice => (
                "Playback Device",
                app.device_settings
                    .selected_output_name()
                    .unwrap_or("No output device"),
                "Press Enter to open the playback device list. Backspace returns only after you enter a list.",
            ),
            SettingsField::RecordingDevice => (
                "Recording Device",
                app.device_settings
                    .selected_input_name()
                    .unwrap_or("No input device"),
                "Press Enter to open the recording device list. Use Backspace there to return here.",
            ),
        };

        let panel = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Current: ", Style::default().fg(Color::Yellow)),
                Span::raw(current),
            ]),
            Line::from(""),
            Line::from(description),
            Line::from(""),
            Line::from("This page now uses a depth-based flow:"),
            Line::from("1. Up/Down picks the setting."),
            Line::from("2. Enter opens its device list."),
            Line::from("3. Enter or Backspace returns from the device list."),
        ])
        .block(Block::default().borders(Borders::ALL).title(title))
        .wrap(Wrap { trim: false });
        frame.render_widget(panel, area);
        return;
    }

    let (title, current, devices) = match app.device_settings.focused_field {
        SettingsField::PlaybackDevice => (
            "Playback Device",
            app.device_settings
                .selected_output_name()
                .unwrap_or("No output device"),
            app.device_settings.output_devices.as_slice(),
        ),
        SettingsField::RecordingDevice => (
            "Recording Device",
            app.device_settings
                .selected_input_name()
                .unwrap_or("No input device"),
            app.device_settings.input_devices.as_slice(),
        ),
    };

    let layout = ratatui::layout::Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([
            ratatui::layout::Constraint::Length(5),
            ratatui::layout::Constraint::Min(0),
            ratatui::layout::Constraint::Length(4),
        ])
        .split(area);

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Focused: ", Style::default().fg(Color::Yellow)),
            Span::raw(title),
        ]),
        Line::from(vec![
            Span::styled("Current: ", Style::default().fg(Color::Yellow)),
            Span::raw(current),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Device Routing"),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(header, layout[0]);

    let items = if devices.is_empty() {
        vec![ListItem::new("No devices found.")]
    } else {
        devices
            .iter()
            .map(|device| ListItem::new(device.as_str()))
            .collect::<Vec<_>>()
    };
    let mut state = ListState::default();
    state.select(app.device_settings.active_selected_index());
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Available Devices"),
        )
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    frame.render_stateful_widget(list, layout[1], &mut state);

    let mut footer_lines = vec![
        Line::from("Up/Down changes the selected device in this list."),
        Line::from("Enter confirms this device and returns to the previous settings level."),
        Line::from("Backspace also returns without leaving the settings page."),
        Line::from("The selected device is used for the next playback or recording session."),
    ];
    if let Some(message) = &app.device_settings.status_message {
        footer_lines.push(Line::from(""));
        footer_lines.push(Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Yellow)),
            Span::raw(message.clone()),
        ]));
    }

    let info = Paragraph::new(footer_lines)
        .block(Block::default().borders(Borders::ALL).title("Info"))
        .wrap(Wrap { trim: false });
    frame.render_widget(info, layout[2]);
}
