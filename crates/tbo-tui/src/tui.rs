//! Terminal setup and teardown.
//!
//! [`init`] switches the terminal into raw mode and the alternate screen and
//! installs a panic hook that restores the terminal before the default hook
//! prints the panic message — otherwise a panic would leave the user's shell in
//! a broken state.

use std::io::{self, Stdout};

use anyhow::Result;
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// The concrete terminal type used throughout the app.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enter raw mode and the alternate screen, install the panic hook, and build
/// the terminal.
pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    set_panic_hook();
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

/// Leave the alternate screen and disable raw mode. Safe to call more than
/// once.
pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Install a panic hook that restores the terminal before delegating to the
/// previously-installed hook.
fn set_panic_hook() {
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore();
        hook(info);
    }));
}
