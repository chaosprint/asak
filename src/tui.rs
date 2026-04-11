use anyhow::{Context, Result};
use chrono::Local;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam::channel::{unbounded, Receiver, Sender};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use dasp_interpolate::linear::Linear;
use dasp_signal::Signal as DaspSignal;
use hound::{SampleFormat as WavSampleFormat, WavSpec, WavWriter};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{CrosstermBackend, Terminal},
    style::{Color, Modifier, Style},
    symbols,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine},
        Block, Borders, List, ListItem, ListState, Paragraph, Wrap,
    },
};
use std::{
    fs::{self, File},
    io::{stdout, Stdout},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
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

const AUDIO_EXTENSIONS: &[&str] = &["wav", "mp3", "ogg"];
const MODE_TITLES: [&str; 3] = ["Play", "Rec", "Settings"];
const SIDEBAR_WIDTH: u16 = 30;
const WAVEFORM_CACHE_BUCKETS: usize = 2048;
const BRAILLE_PIXELS_PER_CELL_X: usize = 2;
const BRAILLE_PIXELS_PER_CELL_Y: usize = 4;
const WAVEFORM_BAR_WIDTH_DOTS: usize = 1;
const WAVEFORM_BAR_GAP_DOTS: usize = 0;
const WAVEFORM_VIEW_SECONDS: f64 = 12.0;
const RECENT_WAVEFORM_VIEW_SECONDS: f64 = 8.0;
const METER_WINDOW_MS: usize = 90;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Play,
    Rec,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavigationLevel {
    TabSelect,
    TabContent,
}

impl ActiveTab {
    fn index(self) -> usize {
        match self {
            Self::Play => 0,
            Self::Rec => 1,
            Self::Settings => 2,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Play => Self::Rec,
            Self::Rec => Self::Settings,
            Self::Settings => Self::Play,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::Play => Self::Settings,
            Self::Rec => Self::Play,
            Self::Settings => Self::Rec,
        }
    }
}

#[derive(Debug, Clone)]
enum BrowserEntryKind {
    Parent,
    Directory,
    AudioFile,
}

#[derive(Debug, Clone)]
struct BrowserEntry {
    name: String,
    path: PathBuf,
    kind: BrowserEntryKind,
}

#[derive(Debug, Clone)]
struct MeterLevel {
    label: String,
    peak: f32,
}

#[derive(Debug, Clone)]
struct AudioPreview {
    name: String,
    path: PathBuf,
    format: String,
    duration: f32,
    sample_rate: f64,
    channels: usize,
    samples: Vec<Vec<f32>>,
    waveform: Vec<f32>,
    meters: Vec<MeterLevel>,
}

#[derive(Debug, Clone)]
enum PlayView {
    Browser,
    Preview(AudioPreview),
    PreviewError { path: PathBuf, message: String },
}

struct PlaybackSession {
    _stream: cpal::Stream,
    is_paused: Arc<AtomicBool>,
    pointer: Arc<AtomicUsize>,
    samples: Arc<Vec<Vec<f32>>>,
    frame_len: usize,
    sample_rate: f64,
}

struct RecordingSession {
    _stream: cpal::Stream,
    rx: Receiver<Vec<f32>>,
    output_path: PathBuf,
    writer_thread: JoinHandle<Result<()>>,
    started_at: Instant,
    sample_rate: f64,
    channels: usize,
    device_name: String,
    recent_samples: Vec<f32>,
    session_samples: Vec<f32>,
}

struct App {
    active_tab: ActiveTab,
    navigation_level: NavigationLevel,
    play_dir: PathBuf,
    play_entries: Vec<BrowserEntry>,
    play_list_state: ListState,
    play_view: PlayView,
    playback_session: Option<PlaybackSession>,
    rec_file_name: String,
    rec_cursor_position: usize,
    recording_session: Option<RecordingSession>,
    recording_error: Option<String>,
    last_recording_path: Option<PathBuf>,
}

impl App {
    fn new() -> Self {
        let rec_file_name = default_recording_name();
        let rec_cursor_position = default_recording_cursor(&rec_file_name);
        let play_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let play_entries = load_browser_entries(&play_dir);
        let mut play_list_state = ListState::default();
        if !play_entries.is_empty() {
            play_list_state.select(Some(0));
        }

        Self {
            active_tab: ActiveTab::Play,
            navigation_level: NavigationLevel::TabSelect,
            play_dir,
            play_entries,
            play_list_state,
            play_view: PlayView::Browser,
            playback_session: None,
            rec_file_name,
            rec_cursor_position,
            recording_session: None,
            recording_error: None,
            last_recording_path: None,
        }
    }

    fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    fn previous_tab(&mut self) {
        self.active_tab = self.active_tab.previous();
    }

    fn enter_active_tab(&mut self) {
        if self.active_tab == ActiveTab::Rec
            && self.recording_session.is_none()
            && self.rec_file_name.is_empty()
        {
            self.reset_recording_name();
        }
        self.navigation_level = NavigationLevel::TabContent;
    }

    fn leave_active_tab(&mut self) {
        if self.active_tab == ActiveTab::Play && self.is_in_play_preview() {
            self.leave_preview();
            return;
        }

        if self.active_tab == ActiveTab::Rec && self.recording_session.is_some() {
            self.stop_recording();
        }

        self.navigation_level = NavigationLevel::TabSelect;
    }

    fn is_in_play_browser(&self) -> bool {
        matches!(self.play_view, PlayView::Browser)
    }

    fn is_in_play_preview(&self) -> bool {
        !self.is_in_play_browser()
    }

    fn select_next_file(&mut self) {
        if self.play_entries.is_empty() {
            self.play_list_state.select(None);
            return;
        }

        let next = match self.play_list_state.selected() {
            Some(index) => (index + 1).min(self.play_entries.len() - 1),
            None => 0,
        };
        self.play_list_state.select(Some(next));
    }

    fn select_previous_file(&mut self) {
        if self.play_entries.is_empty() {
            self.play_list_state.select(None);
            return;
        }

        let previous = match self.play_list_state.selected() {
            Some(index) => index.saturating_sub(1),
            None => 0,
        };
        self.play_list_state.select(Some(previous));
    }

    fn selected_entry(&self) -> Option<&BrowserEntry> {
        self.play_list_state
            .selected()
            .and_then(|index| self.play_entries.get(index))
    }

    fn activate_selected_entry(&mut self) {
        let Some(entry) = self.selected_entry().cloned() else {
            return;
        };

        match entry.kind {
            BrowserEntryKind::Parent | BrowserEntryKind::Directory => {
                self.change_directory(entry.path);
            }
            BrowserEntryKind::AudioFile => {
                self.stop_playback();
                self.play_view = match load_audio_preview(&entry.path) {
                    Ok(preview) => match start_playback(&preview) {
                        Ok(session) => {
                            self.playback_session = Some(session);
                            PlayView::Preview(preview)
                        }
                        Err(err) => PlayView::PreviewError {
                            path: entry.path,
                            message: err.to_string(),
                        },
                    },
                    Err(err) => PlayView::PreviewError {
                        path: entry.path,
                        message: err.to_string(),
                    },
                };
            }
        }
    }

    fn go_to_parent_directory(&mut self) {
        if let Some(parent) = self.play_dir.parent() {
            self.change_directory(parent.to_path_buf());
        }
    }

    fn leave_preview(&mut self) {
        self.stop_playback();
        self.play_view = PlayView::Browser;
    }

    fn change_directory(&mut self, next_dir: PathBuf) {
        self.stop_playback();
        self.play_dir = next_dir;
        self.play_entries = load_browser_entries(&self.play_dir);
        self.play_view = PlayView::Browser;
        if self.play_entries.is_empty() {
            self.play_list_state.select(None);
        } else {
            self.play_list_state.select(Some(0));
        }
    }

    fn stop_playback(&mut self) {
        self.playback_session = None;
    }

    fn toggle_pause(&mut self) {
        if let Some(session) = &self.playback_session {
            let paused = session.is_paused.load(Ordering::Relaxed);
            session.is_paused.store(!paused, Ordering::Relaxed);
        }
    }

    fn is_playback_paused(&self) -> bool {
        self.playback_session
            .as_ref()
            .map(|session| session.is_paused.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    fn playback_snapshot(&self) -> Option<PlaybackSnapshot> {
        self.playback_session
            .as_ref()
            .map(|session| PlaybackSnapshot {
                pointer: session.pointer.load(Ordering::Relaxed),
                frame_len: session.frame_len,
                sample_rate: session.sample_rate,
                samples: session.samples.clone(),
            })
    }

    fn poll_recording(&mut self) {
        let Some(session) = &mut self.recording_session else {
            return;
        };

        while let Ok(chunk) = session.rx.try_recv() {
            let mono = downmix_interleaved(&chunk, session.channels);
            if mono.is_empty() {
                continue;
            }

            session.session_samples.extend_from_slice(&mono);
            session.recent_samples.extend_from_slice(&mono);
        }

        let max_recent_samples =
            (session.sample_rate * RECENT_WAVEFORM_VIEW_SECONDS).max(1.0) as usize;
        if session.recent_samples.len() > max_recent_samples {
            let excess = session.recent_samples.len() - max_recent_samples;
            session.recent_samples.drain(..excess);
        }
    }

    fn start_recording(&mut self) {
        if self.recording_session.is_some() {
            return;
        }

        self.recording_error = None;
        self.last_recording_path = None;

        match start_recording_session(&self.rec_file_name) {
            Ok(session) => self.recording_session = Some(session),
            Err(err) => self.recording_error = Some(err.to_string()),
        }
    }

    fn stop_recording(&mut self) {
        if let Some(session) = self.recording_session.take() {
            let output_path = session.output_path.clone();
            drop(session._stream);

            match session.writer_thread.join() {
                Ok(Ok(())) => {
                    self.recording_error = None;
                    self.last_recording_path = Some(output_path);
                    self.reset_recording_name();
                }
                Ok(Err(err)) => self.recording_error = Some(err.to_string()),
                Err(_) => self.recording_error = Some("Writer thread panicked".to_string()),
            }
        }
    }

    fn reset_recording_name(&mut self) {
        self.rec_file_name = default_recording_name();
        self.rec_cursor_position = default_recording_cursor(&self.rec_file_name);
    }

    fn rec_name_len(&self) -> usize {
        self.rec_file_name.chars().count()
    }

    fn move_rec_cursor_left(&mut self) {
        self.rec_cursor_position = self.rec_cursor_position.saturating_sub(1);
    }

    fn move_rec_cursor_right(&mut self) {
        self.rec_cursor_position = (self.rec_cursor_position + 1).min(self.rec_name_len());
    }

    fn insert_rec_char(&mut self, ch: char) {
        let byte_index = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position);
        self.rec_file_name.insert(byte_index, ch);
        self.rec_cursor_position += 1;
    }

    fn remove_rec_char_before_cursor(&mut self) {
        if self.rec_cursor_position == 0 {
            return;
        }

        let start = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position - 1);
        let end = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position);
        self.rec_file_name.replace_range(start..end, "");
        self.rec_cursor_position -= 1;
    }

    fn remove_rec_char_at_cursor(&mut self) {
        if self.rec_cursor_position >= self.rec_name_len() {
            return;
        }

        let start = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position);
        let end = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position + 1);
        self.rec_file_name.replace_range(start..end, "");
    }

    fn handle_rec_key(&mut self, key: KeyCode) {
        match key {
            KeyCode::Enter if self.recording_session.is_some() => self.stop_recording(),
            KeyCode::Enter => self.start_recording(),
            KeyCode::Left if self.recording_session.is_none() => self.move_rec_cursor_left(),
            KeyCode::Right if self.recording_session.is_none() => self.move_rec_cursor_right(),
            KeyCode::Home if self.recording_session.is_none() => self.rec_cursor_position = 0,
            KeyCode::End if self.recording_session.is_none() => {
                self.rec_cursor_position = self.rec_name_len();
            }
            KeyCode::Backspace if self.recording_session.is_none() => {
                self.remove_rec_char_before_cursor();
            }
            KeyCode::Delete if self.recording_session.is_none() => {
                self.remove_rec_char_at_cursor();
            }
            KeyCode::Char(ch) if self.recording_session.is_none() && !ch.is_control() => {
                self.insert_rec_char(ch);
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
struct PlaybackSnapshot {
    pointer: usize,
    frame_len: usize,
    sample_rate: f64,
    samples: Arc<Vec<Vec<f32>>>,
}

pub fn run() -> Result<()> {
    enable_raw_mode()?;
    let mut terminal_stdout = stdout();
    execute!(terminal_stdout, EnterAlternateScreen)?;

    let result = run_app(&mut Terminal::new(CrosstermBackend::new(terminal_stdout))?);

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;

    result
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut app = App::new();

    loop {
        if app.active_tab == ActiveTab::Rec && app.navigation_level == NavigationLevel::TabContent {
            app.poll_recording();
        }

        terminal.draw(|frame| render(frame.area(), frame, &app))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        match key.code {
            KeyCode::Char('q')
                if !(app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Rec
                    && app.recording_session.is_none()) =>
            {
                break
            }
            KeyCode::Esc if app.navigation_level == NavigationLevel::TabContent => {
                app.leave_active_tab();
            }
            KeyCode::Esc => break,
            KeyCode::Up | KeyCode::Char('k')
                if app.navigation_level == NavigationLevel::TabSelect =>
            {
                app.previous_tab();
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab
                if app.navigation_level == NavigationLevel::TabSelect =>
            {
                app.next_tab();
            }
            KeyCode::Enter if app.navigation_level == NavigationLevel::TabSelect => {
                app.enter_active_tab();
            }
            KeyCode::Up | KeyCode::Char('k')
                if app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Play
                    && app.is_in_play_browser() =>
            {
                app.select_previous_file();
            }
            KeyCode::Down | KeyCode::Char('j')
                if app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Play
                    && app.is_in_play_browser() =>
            {
                app.select_next_file();
            }
            KeyCode::Enter
                if app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Play =>
            {
                app.activate_selected_entry();
            }
            KeyCode::Char(' ')
                if app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Play
                    && app.is_in_play_preview() =>
            {
                app.toggle_pause();
            }
            key if app.navigation_level == NavigationLevel::TabContent
                && app.active_tab == ActiveTab::Rec =>
            {
                app.handle_rec_key(key)
            }
            KeyCode::Backspace
                if app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Play
                    && app.is_in_play_preview() =>
            {
                app.leave_preview();
            }
            KeyCode::Backspace
                if app.navigation_level == NavigationLevel::TabContent
                    && app.active_tab == ActiveTab::Play
                    && app.is_in_play_browser() =>
            {
                app.go_to_parent_directory();
            }
            _ => {}
        }
    }

    Ok(())
}

fn render(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(layout[0]);

    render_sidebar(content[0], frame, app);
    render_main_panel(content[1], frame, app);

    let footer_text = match app.navigation_level {
        NavigationLevel::TabSelect => "up/down: choose mode | enter: open | q/esc: quit",
        NavigationLevel::TabContent => match (&app.active_tab, &app.play_view) {
            (ActiveTab::Play, PlayView::Browser) => {
                "up/down: move | enter: open | backspace: up | esc: modes | q: quit"
            }
            (ActiveTab::Play, _) => {
                "enter: open selected | space: pause | backspace: browser | esc: modes | q: quit"
            }
            (ActiveTab::Rec, _) => {
                "type filename | left/right: move cursor | enter: start/stop | esc: modes | q: quit"
            }
            (ActiveTab::Settings, _) => "esc: modes | q: quit",
        },
    };

    let footer = Paragraph::new(footer_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(footer, layout[1]);
}

fn render_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    match app.navigation_level {
        NavigationLevel::TabSelect => render_mode_sidebar(area, frame, app),
        NavigationLevel::TabContent => match app.active_tab {
            ActiveTab::Play => render_play_sidebar(area, frame, app),
            ActiveTab::Rec => render_rec_sidebar(area, frame, app),
            ActiveTab::Settings => render_settings_sidebar(area, frame),
        },
    }
}

fn render_main_panel(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    match app.navigation_level {
        NavigationLevel::TabSelect => render_mode_description(area, frame, app),
        NavigationLevel::TabContent => match app.active_tab {
            ActiveTab::Play => render_play_tab(area, frame, app),
            ActiveTab::Rec => render_rec_main(area, frame, app),
            ActiveTab::Settings => render_settings_main(area, frame),
        },
    }
}

fn render_mode_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
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

fn render_mode_description(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
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

fn render_play_tab(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
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

fn render_play_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
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
    let (waveform_levels, playhead_index) = if let Some(snapshot) = &playback {
        build_dynamic_waveform_levels(
            &snapshot.samples,
            snapshot.pointer,
            snapshot.sample_rate,
            waveform_bucket_count,
        )
    } else {
        (
            resample_levels(&preview.waveform, waveform_bucket_count),
            waveform_bucket_count / 2,
        )
    };

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
        .map(|snapshot| build_dynamic_meter_levels(snapshot))
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
    .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(info, layout[3]);
}

fn render_play_preview_error(
    area: Rect,
    frame: &mut ratatui::Frame<'_>,
    path: &Path,
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
    .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(error, area);
}

fn render_rec_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
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

fn render_rec_main(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let Some(session) = &app.recording_session else {
        let info = Paragraph::new(vec![
            Line::from("Rec waits for a filename on the left before opening the stream."),
            Line::from("Once recording starts, the right side shows both waveforms."),
            Line::from(""),
            Line::from("Top: rolling live waveform."),
            Line::from("Bottom: cumulative waveform for the whole take."),
        ])
        .block(Block::default().borders(Borders::ALL).title("Rec Overview"))
        .wrap(Wrap { trim: false });
        frame.render_widget(info, area);
        return;
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(12),
            Constraint::Length(6),
            Constraint::Length(4),
        ])
        .split(area);

    let recent_block = Block::default().borders(Borders::ALL).title("Recent");
    let recent_inner = recent_block.inner(layout[0]);
    let recent_dot_width = recent_inner.width as usize * BRAILLE_PIXELS_PER_CELL_X;
    let recent_dot_height = recent_inner.height as usize * BRAILLE_PIXELS_PER_CELL_Y;
    let recent_bucket_count = (recent_dot_width + WAVEFORM_BAR_GAP_DOTS)
        / (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS);
    let recent_levels = build_waveform_cache_mono(&session.recent_samples, recent_bucket_count);

    let recent_waveform = Canvas::default()
        .marker(symbols::Marker::Braille)
        .block(recent_block)
        .x_bounds([0.0, recent_dot_width.max(1) as f64 - 1.0])
        .y_bounds([0.0, recent_dot_height.max(1) as f64 - 1.0])
        .paint(move |ctx| {
            draw_waveform_without_playhead(
                ctx,
                &recent_levels,
                recent_dot_width,
                recent_dot_height,
            );
        });
    frame.render_widget(recent_waveform, layout[0]);

    let session_block = Block::default().borders(Borders::ALL).title("Session");
    let session_inner = session_block.inner(layout[1]);
    let session_dot_width = session_inner.width as usize * BRAILLE_PIXELS_PER_CELL_X;
    let session_dot_height = session_inner.height as usize * BRAILLE_PIXELS_PER_CELL_Y;
    let session_bucket_count = (session_dot_width + WAVEFORM_BAR_GAP_DOTS)
        / (WAVEFORM_BAR_WIDTH_DOTS + WAVEFORM_BAR_GAP_DOTS);
    let session_levels = build_waveform_cache_mono(&session.session_samples, session_bucket_count);
    let session_playhead = session_levels.len().saturating_sub(1);

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

    let info = Paragraph::new(vec![
        Line::from("Top: continuously scrolling recent waveform."),
        Line::from("Bottom: cumulative waveform across the whole recording."),
    ])
    .block(Block::default().borders(Borders::ALL).title("Info"))
    .wrap(Wrap { trim: false });
    frame.render_widget(info, layout[2]);
}

fn render_settings_sidebar(area: Rect, frame: &mut ratatui::Frame<'_>) {
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

fn render_settings_main(area: Rect, frame: &mut ratatui::Frame<'_>) {
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

fn load_browser_entries(root: &Path) -> Vec<BrowserEntry> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut directories = Vec::new();
    let mut audio_files = Vec::new();

    for path in entries.flatten().map(|entry| entry.path()) {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();

        if path.is_dir() {
            directories.push(BrowserEntry {
                name,
                path,
                kind: BrowserEntryKind::Directory,
            });
        } else if path.is_file() && is_supported_audio_file(&path) {
            audio_files.push(BrowserEntry {
                name,
                path,
                kind: BrowserEntryKind::AudioFile,
            });
        }
    }

    directories.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    audio_files.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));

    let mut all_entries = Vec::new();
    if let Some(parent) = root.parent() {
        all_entries.push(BrowserEntry {
            name: "..".to_string(),
            path: parent.to_path_buf(),
            kind: BrowserEntryKind::Parent,
        });
    }
    all_entries.extend(directories);
    all_entries.extend(audio_files);
    all_entries
}

fn is_supported_audio_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn load_audio_preview(path: &Path) -> Result<AudioPreview> {
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
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend_from_slice(buf.chan(ch));
                }
            }
            AudioBufferRef::F64(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(buf.chan(ch).iter().map(|&sample| sample as f32));
                }
            }
            AudioBufferRef::S8(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| sample as f32 / i8::MAX as f32),
                    );
                }
            }
            AudioBufferRef::S16(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| sample as f32 / i16::MAX as f32),
                    );
                }
            }
            AudioBufferRef::S24(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|sample| sample.inner() as f32 / 8_388_607.0),
                    );
                }
            }
            AudioBufferRef::S32(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| sample as f32 / i32::MAX as f32),
                    );
                }
            }
            AudioBufferRef::U8(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| (sample as f32 / u8::MAX as f32) * 2.0 - 1.0),
                    );
                }
            }
            AudioBufferRef::U16(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|&sample| (sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
                    );
                }
            }
            AudioBufferRef::U24(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
                        buf.chan(ch)
                            .iter()
                            .map(|sample| (sample.inner() as f32 / 16_777_215.0) * 2.0 - 1.0),
                    );
                }
            }
            AudioBufferRef::U32(buf) => {
                for ch in 0..buf.spec().channels.count().min(channels) {
                    samples[ch].extend(
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

fn start_playback(preview: &AudioPreview) -> Result<PlaybackSession> {
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

fn start_recording_session(name: &str) -> Result<RecordingSession> {
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

fn recording_output_path(name: &str) -> Result<PathBuf> {
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

fn default_recording_name() -> String {
    Local::now().format("%Y-%m-%d_%H-%M-%S.wav").to_string()
}

fn default_recording_cursor(name: &str) -> usize {
    name.strip_suffix(".wav")
        .map(|stem| stem.chars().count())
        .unwrap_or_else(|| name.chars().count())
}

fn char_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(text.len())
}

fn build_waveform_cache(channels: &[Vec<f32>], bucket_count: usize) -> Vec<f32> {
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

fn build_waveform_cache_mono(samples: &[f32], bucket_count: usize) -> Vec<f32> {
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
        let mut peak = 0.0f32;
        let mut sample_count = 0usize;
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

fn downmix_interleaved(samples: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 || samples.is_empty() {
        return Vec::new();
    }

    samples
        .chunks(channels)
        .map(|frame| frame.iter().copied().sum::<f32>() / frame.len() as f32)
        .collect()
}

fn build_meter_levels(channels: &[Vec<f32>]) -> Vec<MeterLevel> {
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

fn resample_levels(levels: &[f32], bucket_count: usize) -> Vec<f32> {
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

fn build_dynamic_waveform_levels(
    channels: &[Vec<f32>],
    pointer: usize,
    sample_rate: f64,
    bucket_count: usize,
) -> (Vec<f32>, usize) {
    if bucket_count == 0 || channels.is_empty() || channels[0].is_empty() {
        return (Vec::new(), 0);
    }

    let window_frames = ((sample_rate * WAVEFORM_VIEW_SECONDS).round() as usize).max(bucket_count);
    let half_window = window_frames / 2;
    let frame_len = channels[0].len();
    let start = pointer.saturating_sub(half_window);
    let mut levels = Vec::with_capacity(bucket_count);

    for bucket in 0..bucket_count {
        let bucket_start = start + (bucket * window_frames) / bucket_count;
        let mut bucket_end = start + ((bucket + 1) * window_frames) / bucket_count;
        if bucket_end <= bucket_start {
            bucket_end = bucket_start + 1;
        }

        let range_start = bucket_start.min(frame_len.saturating_sub(1));
        let range_end = bucket_end.min(frame_len).max(range_start + 1);
        levels.push(bucket_level(channels, range_start, range_end));
    }

    let playhead_index = (((pointer.saturating_sub(start)) as f32 / window_frames as f32)
        * bucket_count as f32)
        .floor() as usize;

    (
        normalize_and_smooth(&levels),
        playhead_index.min(bucket_count.saturating_sub(1)),
    )
}

fn build_dynamic_meter_levels(snapshot: &PlaybackSnapshot) -> Vec<MeterLevel> {
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

fn bucket_level(channels: &[Vec<f32>], start: usize, end: usize) -> f32 {
    let mut square_sum = 0.0f64;
    let mut sample_count = 0usize;
    let mut peak = 0.0f32;

    for channel in channels {
        if channel.is_empty() {
            continue;
        }
        let range_start = start.min(channel.len().saturating_sub(1));
        let range_end = end.min(channel.len()).max(range_start + 1);
        for &sample in &channel[range_start..range_end] {
            square_sum += (sample as f64) * (sample as f64);
            peak = peak.max(sample.abs());
            sample_count += 1;
        }
    }

    if sample_count == 0 {
        return 0.0;
    }

    let rms = (square_sum / sample_count as f64).sqrt() as f32;
    (rms * 0.84) + (peak * 0.16)
}

fn draw_waveform(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
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

fn draw_waveform_without_playhead(
    ctx: &mut ratatui::widgets::canvas::Context<'_>,
    levels: &[f32],
    dot_width: usize,
    dot_height: usize,
) {
    if levels.is_empty() || dot_width == 0 || dot_height == 0 {
        return;
    }

    let midpoint = (dot_height as f64 - 1.0) / 2.0;
    let max_bar_radius = midpoint.max(1.0);

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

fn render_preview_meter(area: Rect, buf: &mut ratatui::prelude::Buffer, meters: &[MeterLevel]) {
    use ratatui::prelude::Widget;

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

fn format_duration(seconds: f32) -> String {
    let total_seconds = seconds.max(0.0).round() as u64;
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn format_sample_rate(sample_rate: f64) -> String {
    format!("{:.1} kHz", sample_rate / 1000.0)
}
