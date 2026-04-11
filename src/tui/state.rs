use anyhow::Result;
use cpal::Stream;
use crossbeam::channel::Receiver;
use crossterm::event::KeyCode;
use ratatui::widgets::ListState;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::Instant,
};

use crate::tui::{
    audio::{load_audio_preview, load_browser_entries, start_playback, start_recording_session},
    char_to_byte_index, default_recording_cursor, default_recording_name, downmix_interleaved,
    RECENT_WAVEFORM_VIEW_SECONDS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActiveTab {
    Play,
    Rec,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavigationLevel {
    TabSelect,
    TabContent,
}

impl ActiveTab {
    pub(crate) fn index(self) -> usize {
        match self {
            Self::Play => 0,
            Self::Rec => 1,
            Self::Settings => 2,
        }
    }

    pub(crate) fn next(self) -> Self {
        match self {
            Self::Play => Self::Rec,
            Self::Rec => Self::Settings,
            Self::Settings => Self::Play,
        }
    }

    pub(crate) fn previous(self) -> Self {
        match self {
            Self::Play => Self::Settings,
            Self::Rec => Self::Play,
            Self::Settings => Self::Rec,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum BrowserEntryKind {
    Parent,
    Directory,
    AudioFile,
}

#[derive(Debug, Clone)]
pub(crate) struct BrowserEntry {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) kind: BrowserEntryKind,
}

#[derive(Debug, Clone)]
pub(crate) struct MeterLevel {
    pub(crate) label: String,
    pub(crate) peak: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct AudioPreview {
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) format: String,
    pub(crate) duration: f32,
    pub(crate) sample_rate: f64,
    pub(crate) channels: usize,
    pub(crate) samples: Vec<Vec<f32>>,
    pub(crate) waveform: Vec<f32>,
    pub(crate) meters: Vec<MeterLevel>,
}

#[derive(Debug, Clone)]
pub(crate) enum PlayView {
    Browser,
    Preview(AudioPreview),
    PreviewError { path: PathBuf, message: String },
}

pub(crate) struct PlaybackSession {
    pub(crate) _stream: Stream,
    pub(crate) is_paused: Arc<AtomicBool>,
    pub(crate) pointer: Arc<AtomicUsize>,
    pub(crate) samples: Arc<Vec<Vec<f32>>>,
    pub(crate) frame_len: usize,
    pub(crate) sample_rate: f64,
}

pub(crate) struct RecordingSession {
    pub(crate) _stream: Stream,
    pub(crate) rx: Receiver<Vec<f32>>,
    pub(crate) output_path: PathBuf,
    pub(crate) writer_thread: JoinHandle<Result<()>>,
    pub(crate) started_at: Instant,
    pub(crate) sample_rate: f64,
    pub(crate) channels: usize,
    pub(crate) device_name: String,
    pub(crate) recent_samples: Vec<f32>,
    pub(crate) session_samples: Vec<f32>,
}

pub(crate) struct App {
    pub(crate) active_tab: ActiveTab,
    pub(crate) navigation_level: NavigationLevel,
    pub(crate) play_dir: PathBuf,
    pub(crate) play_entries: Vec<BrowserEntry>,
    pub(crate) play_list_state: ListState,
    pub(crate) play_view: PlayView,
    pub(crate) playback_session: Option<PlaybackSession>,
    pub(crate) rec_file_name: String,
    pub(crate) rec_cursor_position: usize,
    pub(crate) recording_session: Option<RecordingSession>,
    pub(crate) recording_error: Option<String>,
    pub(crate) last_recording_path: Option<PathBuf>,
}

impl App {
    pub(crate) fn new() -> Self {
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

    pub(crate) fn next_tab(&mut self) {
        self.active_tab = self.active_tab.next();
    }

    pub(crate) fn previous_tab(&mut self) {
        self.active_tab = self.active_tab.previous();
    }

    pub(crate) fn enter_active_tab(&mut self) {
        if self.active_tab == ActiveTab::Rec
            && self.recording_session.is_none()
            && self.rec_file_name.is_empty()
        {
            self.reset_recording_name();
        }
        self.navigation_level = NavigationLevel::TabContent;
    }

    pub(crate) fn leave_active_tab(&mut self) {
        if self.active_tab == ActiveTab::Play && self.is_in_play_preview() {
            self.leave_preview();
            return;
        }

        if self.active_tab == ActiveTab::Rec && self.recording_session.is_some() {
            self.stop_recording();
        }

        self.navigation_level = NavigationLevel::TabSelect;
    }

    pub(crate) fn is_in_play_browser(&self) -> bool {
        matches!(self.play_view, PlayView::Browser)
    }

    pub(crate) fn is_in_play_preview(&self) -> bool {
        !self.is_in_play_browser()
    }

    pub(crate) fn select_next_file(&mut self) {
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

    pub(crate) fn select_previous_file(&mut self) {
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

    pub(crate) fn selected_entry(&self) -> Option<&BrowserEntry> {
        self.play_list_state
            .selected()
            .and_then(|index| self.play_entries.get(index))
    }

    pub(crate) fn activate_selected_entry(&mut self) {
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

    pub(crate) fn go_to_parent_directory(&mut self) {
        if let Some(parent) = self.play_dir.parent() {
            self.change_directory(parent.to_path_buf());
        }
    }

    pub(crate) fn leave_preview(&mut self) {
        self.stop_playback();
        self.play_view = PlayView::Browser;
    }

    pub(crate) fn change_directory(&mut self, next_dir: PathBuf) {
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

    pub(crate) fn stop_playback(&mut self) {
        self.playback_session = None;
    }

    pub(crate) fn toggle_pause(&mut self) {
        if let Some(session) = &self.playback_session {
            let paused = session.is_paused.load(Ordering::Relaxed);
            session.is_paused.store(!paused, Ordering::Relaxed);
        }
    }

    pub(crate) fn is_playback_paused(&self) -> bool {
        self.playback_session
            .as_ref()
            .map(|session| session.is_paused.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    pub(crate) fn playback_snapshot(&self) -> Option<PlaybackSnapshot> {
        self.playback_session
            .as_ref()
            .map(|session| PlaybackSnapshot {
                pointer: session.pointer.load(Ordering::Relaxed),
                frame_len: session.frame_len,
                sample_rate: session.sample_rate,
                samples: session.samples.clone(),
            })
    }

    pub(crate) fn poll_recording(&mut self) {
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

    pub(crate) fn start_recording(&mut self) {
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

    pub(crate) fn stop_recording(&mut self) {
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

    pub(crate) fn reset_recording_name(&mut self) {
        self.rec_file_name = default_recording_name();
        self.rec_cursor_position = default_recording_cursor(&self.rec_file_name);
    }

    pub(crate) fn rec_name_len(&self) -> usize {
        self.rec_file_name.chars().count()
    }

    pub(crate) fn move_rec_cursor_left(&mut self) {
        self.rec_cursor_position = self.rec_cursor_position.saturating_sub(1);
    }

    pub(crate) fn move_rec_cursor_right(&mut self) {
        self.rec_cursor_position = (self.rec_cursor_position + 1).min(self.rec_name_len());
    }

    pub(crate) fn insert_rec_char(&mut self, ch: char) {
        let byte_index = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position);
        self.rec_file_name.insert(byte_index, ch);
        self.rec_cursor_position += 1;
    }

    pub(crate) fn remove_rec_char_before_cursor(&mut self) {
        if self.rec_cursor_position == 0 {
            return;
        }

        let start = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position - 1);
        let end = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position);
        self.rec_file_name.replace_range(start..end, "");
        self.rec_cursor_position -= 1;
    }

    pub(crate) fn remove_rec_char_at_cursor(&mut self) {
        if self.rec_cursor_position >= self.rec_name_len() {
            return;
        }

        let start = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position);
        let end = char_to_byte_index(&self.rec_file_name, self.rec_cursor_position + 1);
        self.rec_file_name.replace_range(start..end, "");
    }

    pub(crate) fn handle_rec_key(&mut self, key: KeyCode) {
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
pub(crate) struct PlaybackSnapshot {
    pub(crate) pointer: usize,
    pub(crate) frame_len: usize,
    pub(crate) sample_rate: f64,
    pub(crate) samples: Arc<Vec<Vec<f32>>>,
}
