use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::{CrosstermBackend, Terminal};
use std::io::stdout;

mod audio;
mod constants;
mod render;
mod runtime;
mod settings;
mod state;
mod util;
mod waveform;

pub(crate) use constants::*;
pub(crate) use settings::*;
pub(crate) use state::*;
pub(crate) use util::*;
pub(crate) use waveform::*;

pub fn run() -> Result<()> {
    enable_raw_mode()?;
    let mut terminal_stdout = stdout();
    execute!(terminal_stdout, EnterAlternateScreen)?;

    let result = runtime::run_app(&mut Terminal::new(CrosstermBackend::new(terminal_stdout))?);

    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;

    result
}
