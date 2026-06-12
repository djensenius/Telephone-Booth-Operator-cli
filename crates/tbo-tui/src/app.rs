//! Application state and the main event loop.

use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tbo_auth::{InMemoryTokenStore, KeyringTokenStore, SessionManager, TokenStore};
use tbo_core::config::Config;
use tbo_operator_client::OperatorClient;
use tokio::time::Duration;

use crate::auth::{AuthController, AuthPhase};
use crate::data::{
    EventsController, MessagesController, QuestionsController, SessionTokenProvider,
    SessionsController, SharedSession, StatsController, StatusController, SystemController,
};
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
    status: StatusController,
    messages: MessagesController,
    questions: QuestionsController,
    sessions: SessionsController,
    events: EventsController,
    stats: StatsController,
    system: SystemController,
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

        let session = Self::build_session(&config, &mut toasts)?;
        let auth = AuthController::new(Arc::clone(&session))?;
        match auth.phase() {
            AuthPhase::SignedIn { .. } => toasts.info("Signed in to the operator API."),
            _ => toasts.warn("Not signed in. Open Settings and press L to log in."),
        }

        let api: crate::data::OperatorApi = OperatorClient::new(
            config.operator.base_url.clone(),
            SessionTokenProvider::new(session),
        )?;
        let status = StatusController::new(api.clone());
        let messages = MessagesController::new(api.clone());
        let questions = QuestionsController::new(api.clone());
        let sessions = SessionsController::new(api.clone());

        Ok(Self {
            config,
            theme,
            screen: Screen::Status,
            toasts,
            auth,
            status,
            messages,
            questions,
            sessions,
            events: EventsController::new(api.clone()),
            stats: StatsController::new(api.clone()),
            system: SystemController::new(api),
            should_quit: false,
        })
    }

    /// Build the shared session manager, preferring the OS keychain and falling
    /// back to an ephemeral in-memory store if secure storage is unavailable
    /// (e.g. no secret service on a headless host, or a locked keychain).
    fn build_session(config: &Config, toasts: &mut Toasts) -> Result<SharedSession> {
        match Self::keyring_session(config) {
            Ok(session) => Ok(session),
            Err(err) => {
                toasts.warn(format!(
                    "Secure storage unavailable ({err}); using an in-memory session this run."
                ));
                let store: Box<dyn TokenStore> = Box::new(InMemoryTokenStore::new());
                let manager = SessionManager::new(&config.auth, store)?;
                Ok(Arc::new(manager))
            }
        }
    }

    /// Build a keychain-backed session manager, surfacing any keychain
    /// construction or initial-load error to the caller so the fallback can
    /// engage.
    fn keyring_session(config: &Config) -> Result<SharedSession> {
        let store: Box<dyn TokenStore> = Box::new(KeyringTokenStore::new()?);
        let manager = SessionManager::new(&config.auth, store)?;
        // Probe the store now so a keychain read failure triggers the fallback
        // rather than surfacing later on first use.
        manager.current_session()?;
        Ok(Arc::new(manager))
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

    /// The booth-status controller (drives the Status screen).
    #[must_use]
    pub fn status(&self) -> &StatusController {
        &self.status
    }

    /// The messages controller (drives the Messages screen).
    #[must_use]
    pub fn messages(&self) -> &MessagesController {
        &self.messages
    }

    /// The questions controller (drives the Questions screen).
    #[must_use]
    pub fn questions(&self) -> &QuestionsController {
        &self.questions
    }

    /// The sessions controller (drives the Sessions screen).
    #[must_use]
    pub fn sessions(&self) -> &SessionsController {
        &self.sessions
    }

    /// The events controller (drives the Events screen).
    #[must_use]
    pub fn events(&self) -> &EventsController {
        &self.events
    }

    /// The statistics controller (drives the Stats screen).
    #[must_use]
    pub fn stats(&self) -> &StatsController {
        &self.stats
    }

    /// The live system controller (drives the Live System screen).
    #[must_use]
    pub fn system(&self) -> &SystemController {
        &self.system
    }

    /// Run the draw/event loop until the user quits.
    pub async fn run(mut self, terminal: &mut Tui) -> Result<()> {
        let mut events = EventLoop::new(TICK);
        while !self.should_quit {
            terminal.draw(|frame| ui::render(&self, frame))?;
            match events.next().await {
                AppEvent::Tick => {
                    self.auth.drain(&mut self.toasts);
                    self.status.tick(self.screen == Screen::Status);
                    self.messages.tick(self.screen == Screen::Messages);
                    self.questions.tick(self.screen == Screen::Questions);
                    self.sessions.tick(self.screen == Screen::Sessions);
                    self.events.tick(self.screen == Screen::Events);
                    self.stats.tick(self.screen == Screen::Stats);
                    self.system.tick(self.screen == Screen::LiveSystem);
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
            KeyCode::Down | KeyCode::Char('j') => self.select_next_active(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev_active(),
            KeyCode::Char('r' | 'R') => self.refresh_active(),
            KeyCode::Char('w' | 'W') if self.screen == Screen::Stats => self.stats.cycle_window(),
            KeyCode::Char('f' | 'F') if self.screen == Screen::Events => {
                self.events.toggle_follow();
            }
            KeyCode::Char('l' | 'L') if self.screen == Screen::Settings => self.begin_login(),
            KeyCode::Char('o' | 'O') if self.screen == Screen::Settings => {
                self.auth.sign_out(&mut self.toasts);
            }
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => self.jump_to_digit(c),
            _ => {}
        }
    }

    /// Refresh the data backing the active screen, if it has any.
    fn refresh_active(&mut self) {
        match self.screen {
            Screen::Status => self.status.refresh(),
            Screen::Messages => self.messages.refresh(),
            Screen::Questions => self.questions.refresh(),
            Screen::Sessions => self.sessions.refresh(),
            Screen::Events => self.events.refresh(),
            Screen::Stats => self.stats.refresh(),
            Screen::LiveSystem => self.system.refresh(),
            _ => {}
        }
    }

    /// Advance the selection on the active list screen, if any.
    fn select_next_active(&mut self) {
        match self.screen {
            Screen::Messages => self.messages.select_next(),
            Screen::Questions => self.questions.select_next(),
            Screen::Sessions => self.sessions.select_next(),
            Screen::Events => self.events.select_next(),
            _ => {}
        }
    }

    /// Retreat the selection on the active list screen, if any.
    fn select_prev_active(&mut self) {
        match self.screen {
            Screen::Messages => self.messages.select_prev(),
            Screen::Questions => self.questions.select_prev(),
            Screen::Sessions => self.sessions.select_prev(),
            Screen::Events => self.events.select_prev(),
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
