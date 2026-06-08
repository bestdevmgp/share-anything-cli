pub mod app;
pub mod event;
pub mod widgets;
pub mod screens;

use crate::config::CliConfig;
use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self};

pub async fn run(cfg: CliConfig) -> Result<()> {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen);
        let _ = crossterm::terminal::disable_raw_mode();
        prev_hook(info);
    }));

    let pending: Vec<String> = {
        let _guard = TerminalGuard::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        let mut app = app::App::new(cfg)?;
        event::run_loop(&mut terminal, &mut app).await?;
        app.drain_stdout()
    };
    for line in pending {
        println!("{}", line);
    }
    Ok(())
}

struct TerminalGuard;

impl TerminalGuard {
    #[must_use = "the TerminalGuard must be held for the lifetime of the TUI; dropping it immediately restores the terminal"]
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        let _ = disable_raw_mode();
    }
}

#[allow(unused_variables)]
pub fn copy_to_clipboard(s: &str) -> bool {
    #[cfg(feature = "clipboard")]
    {
        match arboard::Clipboard::new() {
            Ok(mut clip) => clip.set_text(s.to_string()).is_ok(),
            Err(_) => false,
        }
    }
    #[cfg(not(feature = "clipboard"))]
    {
        false
    }
}
