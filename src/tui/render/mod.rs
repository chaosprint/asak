mod play;
mod rec;
mod sidebar;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::Paragraph,
};

use crate::tui::{ActiveTab, App, NavigationLevel, PlayView, SIDEBAR_WIDTH};

pub(crate) fn render(area: Rect, frame: &mut ratatui::Frame<'_>, app: &App) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(area);
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(0)])
        .split(layout[0]);

    sidebar::render_sidebar(content[0], frame, app);
    sidebar::render_main_panel(content[1], frame, app);

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

pub(super) use play::{render_play_sidebar, render_play_tab};
pub(super) use rec::{render_rec_main, render_rec_sidebar};
