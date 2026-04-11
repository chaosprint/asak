use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use ratatui::prelude::{CrosstermBackend, Terminal};
use std::{io::Stdout, time::Duration};

use crate::tui::{render, ActiveTab, App, NavigationLevel};

pub(crate) fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut app = App::new();

    loop {
        app.poll_recording_stop();

        if app.active_tab == ActiveTab::Rec && app.navigation_level == NavigationLevel::TabContent {
            app.poll_recording();
        }

        terminal.draw(|frame| render::render(frame.area(), frame, &app))?;

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
                && app.active_tab == ActiveTab::Settings =>
            {
                app.handle_settings_key(key)
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
