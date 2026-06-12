//! Application state and the main event loop.

use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tbo_auth::{InMemoryTokenStore, KeyringTokenStore, SessionManager, TokenStore};
use tbo_core::config::Config;
use tokio::time::Duration;

use crate::auth::{AuthController, AuthPhase};
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
    auth: AuthController,
    should_quit: bool,
}

impl App {
    /// Build the application from a loaded configuration.
    ///
    /// # Errors
    /// Returns an error if the authentication client or session store cannot be
    /// initialized.
    pub fn new(config: Config) -> Result<Self> {
        let theme = Theme::from_name(&config.ui.theme);
        let mut toasts = Toasts::default();
        toasts.info("Welcome to tb-operator. Press q to quit.");
        if config.booths.is_empty() {
            toasts.warn("No booths configured; add one in Settings to use System Health.");
        }

        let auth = Self::build_auth(&config, &mut toasts)?;
        match auth.phase() {
            AuthPhase::SignedIn { .. } => toasts.info("Signed in to the operator API."),
            _ => toasts.warn("Not signed in. Open Settings and press L to log in."),
        }

        Ok(Self {
            config,
            theme,
            screen: Screen::Status,
            toasts,
            auth,
            should_quit: false,
        })
    }

    /// Build the auth controller, preferring the OS keychain and falling back
    /// to an ephemeral in-memory store if secure storage is unavailable (e.g.
    /// no secret service on a headless host, or a locked keychain).
    fn build_auth(config: &Config, toasts: &mut Toasts) -> Result<AuthController> {
        match Self::keyring_auth(config) {
            Ok(auth) => Ok(auth),
            Err(err) => {
                toasts.warn(format!(
                    "Secure storage unavailable ({err}); using an in-memory session this run."
                ));
                let store: Box<dyn TokenStore> = Box::new(InMemoryTokenStore::new());
                let manager = SessionManager::new(&config.auth, store)?;
                Ok(AuthController::new(Arc::new(manager))?)
            }
        }
    }

    /// Build a keychain-backed auth controller, surfacing any keychain
    /// construction or initial-load error to the caller.
    fn keyring_auth(config: &Config) -> Result<AuthController> {
        let store: Box<dyn TokenStore> = Box::new(KeyringTokenStore::new()?);
        let manager = SessionManager::new(&config.auth, store)?;
        Ok(AuthController::new(Arc::new(manager))?)
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

    /// The authentication controller (drives the login UI).
    #[must_use]
    pub fn auth(&self) -> &AuthController {
        &self.auth
    }

    /// Run the draw/event loop until the user quits.
    pub async fn run(mut self, terminal: &mut Tui) -> Result<()> {
        let mut events = EventLoop::new(TICK);
        while !self.should_quit {
            terminal.draw(|frame| ui::render(&self, frame))?;
            match events.next().await {
                AppEvent::Tick => {
                    self.auth.drain(&mut self.toasts);
                    self.toasts.prune();
                }
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
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                if self.auth.is_in_progress() {
                    self.auth.cancel();
                    self.toasts.info("Login cancelled.");
                } else {
                    self.should_quit = true;
                }
            }
            KeyCode::Tab | KeyCode::Right => self.screen = self.screen.next(),
            KeyCode::BackTab | KeyCode::Left => self.screen = self.screen.prev(),
            KeyCode::Char('l' | 'L') if self.screen == Screen::Settings => self.begin_login(),
            KeyCode::Char('o' | 'O') if self.screen == Screen::Settings => {
                self.auth.sign_out(&mut self.toasts);
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => self.jump_to_digit(c),
            _ => {}
        }
    }

    /// Start a device-code login, nudging the user when it is a no-op.
    fn begin_login(&mut self) {
        match self.auth.phase() {
            AuthPhase::SignedIn { .. } => {
                self.toasts
                    .info("Already signed in; press O to sign out first.");
            }
            AuthPhase::Starting | AuthPhase::AwaitingApproval(_) => {}
            _ => self.auth.start_login(),
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
