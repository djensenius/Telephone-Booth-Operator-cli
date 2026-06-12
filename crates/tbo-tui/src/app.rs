//! Application state and the main event loop.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tbo_core::config::Config;
use tokio::time::Duration;

use crate::event::{AppEvent, EventLoop};
use crate::tui::Tui;
use crate::ui;
use crate::ui::screens::Screen;
use crate::ui::theme::Theme;
use crate::ui::toast::Toasts;

/// How often the UI ticks (drives toast expiry and, later, polling cadence).
const TICK: Duration = Duration::from_millis(250);

/// The running application.
pub struct App {
    config: Config,
    theme: Theme,
    screen: Screen,
    toasts: Toasts,
    should_quit: bool,
}

impl App {
    /// Build the application from a loaded configuration.
    #[must_use]
    pub fn new(config: Config) -> Self {
        let theme = Theme::from_name(&config.ui.theme);
        let mut toasts = Toasts::default();
        toasts.info("Welcome to tb-operator. Press q to quit.");
        if config.booths.is_empty() {
            toasts.warn("No booths configured; add one in Settings to use System Health.");
        }
        Self {
            config,
            theme,
            screen: Screen::Status,
            toasts,
            should_quit: false,
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// The active color theme.
    #[must_use]
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The currently focused screen.
    #[must_use]
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// The active toast queue.
    #[must_use]
    pub fn toasts(&self) -> &Toasts {
        &self.toasts
    }

    /// Run the draw/event loop until the user quits.
    pub async fn run(mut self, terminal: &mut Tui) -> Result<()> {
        let mut events = EventLoop::new(TICK);
        while !self.should_quit {
            terminal.draw(|frame| ui::render(&self, frame))?;
            match events.next().await {
                AppEvent::Tick => self.toasts.prune(),
                AppEvent::Key(key) => self.on_key(key),
                AppEvent::Resize => {}
                AppEvent::Error(message) => self.toasts.error(format!("input error: {message}")),
            }
        }
        Ok(())
    }

    /// Handle a key press.
    fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Tab | KeyCode::Right => self.screen = self.screen.next(),
            KeyCode::BackTab | KeyCode::Left => self.screen = self.screen.prev(),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => self.jump_to_digit(c),
            _ => {}
        }
    }

    /// Jump to the screen addressed by a `1`-`9` digit key, if it exists.
    fn jump_to_digit(&mut self, c: char) {
        let Some(digit) = c.to_digit(10) else { return };
        let Ok(index) = usize::try_from(digit.saturating_sub(1)) else {
            return;
        };
        if let Some(screen) = Screen::from_index(index) {
            self.screen = screen;
        }
    }
}
