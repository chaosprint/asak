use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::tui::{render, ActiveTab, App, NavigationLevel, MODE_TITLES};

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
            ActiveTab::Settings => render_settings_sidebar(area, frame),
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
            ActiveTab::Settings => render_settings_main(area, frame),
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
            vec![
                Line::from("Type a filename in the left column before recording starts."),
                Line::from("The right side shows the rolling waveform and full-session overview."),
                Line::from(""),
                Line::from(Span::styled(
                    "Enter opens the recorder input box in the left pane.",
                    Style::default().fg(Color::Yellow),
                )),
            ],
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

pub(crate) fn render_settings_sidebar(area: ratatui::layout::Rect, frame: &mut ratatui::Frame<'_>) {
    let panel = Paragraph::new(vec![
        Line::from(Span::styled(
            "Settings",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("Device routing"),
        Line::from("Theme / meters"),
        Line::from("Output paths"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Settings"))
    .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}

pub(crate) fn render_settings_main(area: ratatui::layout::Rect, frame: &mut ratatui::Frame<'_>) {
    let panel = Paragraph::new(vec![
        Line::from("Settings is a placeholder for the next round of configuration work."),
        Line::from(""),
        Line::from("Suggested next additions:"),
        Line::from("- input/output device selection"),
        Line::from("- default recording directory"),
        Line::from("- meter and waveform display options"),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Settings Overview"),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(panel, area);
}
