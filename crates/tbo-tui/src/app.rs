//! Application state and the main event loop.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tbo_auth::{InMemoryTokenStore, KeyringTokenStore, SessionManager, TokenStore};
use tbo_core::config::Config;
use tbo_operator_client::OperatorClient;
use tokio::time::Duration;

use crate::auth::{AuthController, AuthPhase};
use crate::data::{
    DebugController, EventsController, MessagesController, PlaybackController, QuestionsController,
    SessionTokenProvider, SessionsController, SharedSession, StatsController, StatusController,
    SystemController, SystemHealthController, TokensController,
};
use crate::event::{AppEvent, EventLoop};
use crate::tui::Tui;
use crate::ui;
use crate::ui::modal::{Intent, Modal, ModalDecision};
use crate::ui::screens::Screen;
use crate::ui::theme::Theme;
use crate::ui::toast::Toasts;

/// How often the UI ticks (drives toast expiry and, later, polling cadence).
const TICK: Duration = Duration::from_millis(250);

/// The running application.
pub struct App {
    config: Config,
    config_path: PathBuf,
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
    system_health: Option<SystemHealthController>,
    debug: Option<DebugController>,
    tokens: TokensController,
    playback: PlaybackController,
    modal: Option<Modal>,
    should_quit: bool,
}

impl App {
    /// Build the application from a loaded configuration and the path it was
    /// loaded from (used when persisting preference changes such as the theme).
    ///
    /// # Errors
    /// Returns an error if the authentication client or session store cannot be
    /// initialized.
    pub fn new(config: Config, config_path: PathBuf) -> Result<Self> {
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

        let system_health = Self::build_system_health(&config, &mut toasts);
        let debug = Self::build_debug(&config, &mut toasts);

        let playback = PlaybackController::new(api.clone());
        if let Some(reason) = playback.unavailable_reason() {
            toasts.warn(format!("Audio playback unavailable: {reason}"));
        }

        Ok(Self {
            config,
            config_path,
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
            system: SystemController::new(api.clone()),
            system_health,
            debug,
            tokens: TokensController::new(api),
            playback,
            modal: None,
            should_quit: false,
        })
    }

    /// Build the System Health controller for the first configured booth, if
    /// any. A construction failure is surfaced as a warning toast and leaves the
    /// dashboard disabled rather than failing startup.
    fn build_system_health(config: &Config, toasts: &mut Toasts) -> Option<SystemHealthController> {
        let booth = config.booths.first()?;
        match SystemHealthController::from_config(booth) {
            Ok(controller) => Some(controller),
            Err(err) => {
                toasts.warn(format!(
                    "System Health unavailable for booth {}: {err}",
                    booth.id
                ));
                None
            }
        }
    }

    /// Build the Debug-panel controller for the first configured booth, if any.
    /// A construction failure is surfaced as a warning toast and leaves the
    /// panel disabled.
    fn build_debug(config: &Config, toasts: &mut Toasts) -> Option<DebugController> {
        let booth = config.booths.first()?;
        match DebugController::from_config(booth) {
            Ok(controller) => Some(controller),
            Err(err) => {
                toasts.warn(format!(
                    "Debug panel unavailable for booth {}: {err}",
                    booth.id
                ));
                None
            }
        }
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

    /// The System Health controller (drives the btm-style dashboard), present
    /// only when a booth is configured.
    #[must_use]
    pub fn system_health(&self) -> Option<&SystemHealthController> {
        self.system_health.as_ref()
    }

    /// The Debug-panel controller (drives the booth Debug screen), present only
    /// when a booth is configured.
    #[must_use]
    pub fn debug(&self) -> Option<&DebugController> {
        self.debug.as_ref()
    }

    /// The API-tokens controller (drives the Tokens screen).
    #[must_use]
    pub fn tokens(&self) -> &TokensController {
        &self.tokens
    }

    /// The audio playback controller (drives the Messages/Questions playback
    /// indicator).
    #[must_use]
    pub fn playback(&self) -> &PlaybackController {
        &self.playback
    }

    /// The active modal overlay, when one is open.
    #[must_use]
    pub fn modal(&self) -> Option<&Modal> {
        self.modal.as_ref()
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
                    self.drain_message_actions();
                    self.questions.tick(self.screen == Screen::Questions);
                    self.drain_question_actions();
                    self.sessions.tick(self.screen == Screen::Sessions);
                    self.events.tick(self.screen == Screen::Events);
                    self.stats.tick(self.screen == Screen::Stats);
                    self.system.tick(self.screen == Screen::LiveSystem);
                    if let Some(health) = self.system_health.as_mut() {
                        health.tick(self.screen == Screen::SystemHealth);
                    }
                    if let Some(debug) = self.debug.as_mut() {
                        debug.tick(self.screen == Screen::Debug);
                    }
                    self.drain_debug_actions();
                    self.tokens.tick(self.screen == Screen::Tokens);
                    self.drain_token_actions();
                    self.drain_playback();
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
        // An open modal captures all other input until it is dismissed.
        if self.modal.is_some() {
            self.handle_modal_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc => {
                if self.screen == Screen::Tokens && self.tokens.dismiss_revealed() {
                    self.toasts.info("Secret dismissed.");
                } else if self.auth.is_in_progress() {
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
            KeyCode::Char('a' | 'A') if self.screen == Screen::Messages => {
                self.message_action(MessagesController::approve_selected);
            }
            KeyCode::Char('x' | 'X') if self.screen == Screen::Messages => {
                self.message_action(MessagesController::reject_selected);
            }
            KeyCode::Char('t' | 'T') if self.screen == Screen::Messages => {
                self.message_action(MessagesController::retranscribe_selected);
            }
            KeyCode::Char('m' | 'M') if self.screen == Screen::Messages => {
                self.message_action(MessagesController::remoderate_selected);
            }
            KeyCode::Char('g' | 'G') if self.screen == Screen::Messages => {
                self.open_translation_prompt();
            }
            KeyCode::Char('d' | 'D') if self.screen == Screen::Messages => {
                self.open_delete_confirm();
            }
            KeyCode::Char('a' | 'A') if self.screen == Screen::Questions => {
                self.question_action(QuestionsController::activate_selected);
            }
            KeyCode::Char('e' | 'E') if self.screen == Screen::Questions => {
                self.question_action(QuestionsController::deactivate_selected);
            }
            KeyCode::Char('d' | 'D') if self.screen == Screen::Questions => {
                self.open_archive_confirm();
            }
            KeyCode::Char('n' | 'N') if self.screen == Screen::Questions => {
                self.open_new_question_prompt();
            }
            KeyCode::Char('p' | 'P')
                if matches!(self.screen, Screen::Messages | Screen::Questions) =>
            {
                self.play_selected_audio();
            }
            KeyCode::Char(' ') if matches!(self.screen, Screen::Messages | Screen::Questions) => {
                self.toggle_playback();
            }
            KeyCode::Char('s' | 'S')
                if matches!(self.screen, Screen::Messages | Screen::Questions) =>
            {
                self.stop_playback();
            }
            KeyCode::Char('n' | 'N') if self.screen == Screen::Tokens => {
                self.open_new_token_prompt();
            }
            KeyCode::Char('d' | 'D') if self.screen == Screen::Tokens => {
                self.open_revoke_confirm();
            }
            KeyCode::Char('u' | 'U') if self.screen == Screen::Tokens => {
                self.tokens.load_usage_selected();
            }
            KeyCode::Char('v' | 'V') if self.screen == Screen::Debug => {
                if let Some(debug) = self.debug.as_mut() {
                    debug.cycle_log_level();
                    self.toasts
                        .info(format!("Log level: {}", debug.log_level()));
                }
            }
            KeyCode::Char('f' | 'F') if self.screen == Screen::Debug => {
                if let Some(debug) = self.debug.as_mut() {
                    debug.toggle_live();
                    if debug.is_live() {
                        self.toasts.info("Live telemetry on.");
                    } else {
                        self.toasts.info("Live telemetry off.");
                    }
                }
            }
            KeyCode::Char('o' | 'O') if self.screen == Screen::Debug => {
                self.debug_simulate(DebugController::simulate_hook_off);
            }
            KeyCode::Char('h' | 'H') if self.screen == Screen::Debug => {
                self.debug_simulate(DebugController::simulate_hook_on);
            }
            KeyCode::Char('p' | 'P') if self.screen == Screen::Debug => {
                self.debug_simulate(DebugController::simulate_playback_ended);
            }
            KeyCode::Char('d' | 'D') if self.screen == Screen::Debug => {
                self.open_dial_prompt();
            }
            KeyCode::Char('l' | 'L') if self.screen == Screen::Settings => self.begin_login(),
            KeyCode::Char('o' | 'O') if self.screen == Screen::Settings => {
                self.auth.sign_out(&mut self.toasts);
            }
            KeyCode::Char('t' | 'T') if self.screen == Screen::Settings => self.cycle_theme(),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => self.jump_to_digit(c),
            _ => {}
        }
    }

    /// Route a key to the active modal, dispatching its action on confirmation.
    fn handle_modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.as_mut() else {
            return;
        };
        match modal.on_key(key) {
            ModalDecision::Stay => {}
            ModalDecision::Cancel => self.modal = None,
            ModalDecision::Confirm => {
                if let Some(modal) = self.modal.take() {
                    self.dispatch_modal(&modal);
                }
            }
        }
    }

    /// Perform the action a confirmed modal represents.
    fn dispatch_modal(&mut self, modal: &Modal) {
        match modal.intent() {
            Intent::DeleteMessage => self.message_action(MessagesController::delete_selected),
            Intent::TranslateMessage => {
                let text = modal.input().to_owned();
                self.message_action(move |m| m.submit_translation(text));
            }
            Intent::ArchiveQuestion => self.question_action(QuestionsController::archive_selected),
            Intent::NewQuestionPrompt => {
                // First create step done; collect the audio path next, carrying
                // the prompt text into the second prompt.
                let prompt = modal.input().to_owned();
                self.modal = Some(Modal::prompt(
                    "New question",
                    "FLAC file path",
                    Intent::NewQuestionAudio { prompt },
                ));
            }
            Intent::NewQuestionAudio { prompt } => {
                let path = modal.input().to_owned();
                self.start_question_create(prompt, path);
            }
            Intent::NewApiToken => {
                let name = modal.input().to_owned();
                self.start_token_create(name);
            }
            Intent::RevokeApiToken => self.tokens.revoke_selected(),
            Intent::SimulateDial => {
                let input = modal.input().to_owned();
                self.simulate_dial(&input);
            }
        }
    }

    /// Run a message write action, guarding against overlapping requests and
    /// nudging when there is nothing selected.
    fn message_action(&mut self, action: impl FnOnce(&mut MessagesController)) {
        if self.messages.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        if self.messages.selected_message().is_none() {
            self.toasts.info("Select a message first.");
            return;
        }
        action(&mut self.messages);
    }

    /// Open a confirmation modal for deleting the selected message.
    fn open_delete_confirm(&mut self) {
        if self.messages.selected_message().is_none() {
            self.toasts.info("Select a message first.");
            return;
        }
        self.modal = Some(Modal::confirm(
            "Delete message",
            "Permanently delete the selected message? This cannot be undone.",
            Intent::DeleteMessage,
        ));
    }

    /// Open a text prompt for submitting a translation of the selected message.
    fn open_translation_prompt(&mut self) {
        if self.messages.selected_message().is_none() {
            self.toasts.info("Select a message first.");
            return;
        }
        self.modal = Some(Modal::prompt(
            "Submit translation",
            "English translation",
            Intent::TranslateMessage,
        ));
    }

    /// Surface completed message-action outcomes as toasts, reloading the list
    /// after a successful change.
    fn drain_message_actions(&mut self) {
        for outcome in self.messages.drain_actions() {
            if outcome.ok {
                self.toasts.info(outcome.message);
                self.messages.refresh();
            } else {
                self.toasts.error(outcome.message);
            }
        }
    }

    /// Run a question write action, guarding against overlapping requests and
    /// nudging when there is nothing selected.
    fn question_action(&mut self, action: impl FnOnce(&mut QuestionsController)) {
        if self.questions.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        if self.questions.selected_question().is_none() {
            self.toasts.info("Select a question first.");
            return;
        }
        action(&mut self.questions);
    }

    /// Open a confirmation modal for archiving the selected question.
    fn open_archive_confirm(&mut self) {
        if self.questions.selected_question().is_none() {
            self.toasts.info("Select a question first.");
            return;
        }
        self.modal = Some(Modal::confirm(
            "Archive question",
            "Retire the selected question? The booth will stop offering it, but existing messages stay on file.",
            Intent::ArchiveQuestion,
        ));
    }

    /// Open the first step of the new-question flow (the prompt text); the
    /// second step collects the FLAC file path.
    fn open_new_question_prompt(&mut self) {
        if self.questions.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        self.modal = Some(Modal::prompt(
            "New question",
            "Prompt text",
            Intent::NewQuestionPrompt,
        ));
    }

    /// Kick off a question create from collected prompt text and audio path,
    /// guarding against overlapping requests.
    fn start_question_create(&mut self, prompt: String, audio_path: String) {
        if self.questions.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        self.questions.create_question(prompt, audio_path);
    }

    /// Surface completed question-action outcomes as toasts, reloading the list
    /// after a successful change.
    fn drain_question_actions(&mut self) {
        for outcome in self.questions.drain_actions() {
            if outcome.ok {
                self.toasts.info(outcome.message);
                self.questions.refresh();
            } else {
                self.toasts.error(outcome.message);
            }
        }
    }

    /// Open a text prompt for naming a new API token.
    fn open_new_token_prompt(&mut self) {
        if self.tokens.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        self.modal = Some(Modal::prompt(
            "New API token",
            "Token name",
            Intent::NewApiToken,
        ));
    }

    /// Kick off a token create from the entered name, guarding against
    /// overlapping requests.
    fn start_token_create(&mut self, name: String) {
        if self.tokens.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        self.tokens.create_token(name);
    }

    /// Open a confirmation modal for revoking the selected token.
    fn open_revoke_confirm(&mut self) {
        if self.tokens.is_action_in_flight() {
            self.toasts.warn("An action is already in progress.");
            return;
        }
        if self.tokens.selected_token().is_none() {
            self.toasts.info("Select a token first.");
            return;
        }
        self.modal = Some(Modal::confirm(
            "Revoke token",
            "Revoke the selected API token? Any client using it will stop working immediately.",
            Intent::RevokeApiToken,
        ));
    }

    /// Surface completed token-action outcomes as toasts, reloading the list
    /// after a successful change.
    fn drain_token_actions(&mut self) {
        for outcome in self.tokens.drain_actions() {
            if outcome.ok {
                self.toasts.info(outcome.message);
                self.tokens.refresh();
            } else {
                self.toasts.error(outcome.message);
            }
        }
    }

    /// Surface the outcome of a finished audio fetch. The controller only emits
    /// an outcome when a download or refresh fails, so this is error-only in
    /// practice, but both arms are handled defensively.
    fn drain_playback(&mut self) {
        if let Some(outcome) = self.playback.drain() {
            if outcome.ok {
                self.toasts.info(outcome.message);
            } else {
                self.toasts.error(outcome.message);
            }
        }
    }

    /// Start playback of the audio attached to the currently selected message or
    /// question. No-ops with a toast when audio output is unavailable, a fetch
    /// is already in flight, or nothing is selected.
    fn play_selected_audio(&mut self) {
        if !self.playback.is_available() {
            let reason = self
                .playback
                .unavailable_reason()
                .unwrap_or("audio output is unavailable")
                .to_string();
            self.toasts.warn(format!("Cannot play audio: {reason}"));
            return;
        }
        if self.playback.is_loading() {
            self.toasts.info("Audio is already loading.");
            return;
        }
        match self.screen {
            Screen::Messages => {
                if let Some(message) = self.messages.selected_message().cloned() {
                    self.playback.play_message(&message);
                    self.toasts.info("Loading message audio…");
                } else {
                    self.toasts.info("Select a message to play its audio.");
                }
            }
            Screen::Questions => {
                if let Some(question) = self.questions.selected_question().cloned() {
                    self.playback.play_question(&question);
                    self.toasts.info("Loading question audio…");
                } else {
                    self.toasts.info("Select a question to play its audio.");
                }
            }
            _ => {}
        }
    }

    /// Toggle between paused and playing for any in-progress playback.
    fn toggle_playback(&self) {
        if self.playback.is_available() {
            self.playback.toggle_pause();
        }
    }

    /// Stop any in-progress playback and clear the player queue.
    fn stop_playback(&self) {
        if self.playback.is_available() {
            self.playback.stop();
        }
    }

    /// Switch to the next color theme, applying it immediately and persisting
    /// the choice to the config file.
    fn cycle_theme(&mut self) {
        let next = Theme::next_name(&self.config.ui.theme);
        self.config.ui.theme = next.to_owned();
        self.theme = Theme::from_name(next);
        match self.config.save_to(&self.config_path) {
            Ok(()) => self.toasts.info(format!("Theme: {next}")),
            Err(err) => self.toasts.warn(format!(
                "Theme set to {next}, but saving config failed: {err}"
            )),
        }
    }

    /// Run a booth simulate action, gating on booth availability, the booth's
    /// `allowControls` setting, and any in-flight request.
    fn debug_simulate(&mut self, action: impl FnOnce(&mut DebugController)) {
        if self.debug.is_none() {
            self.toasts.info("No booth is configured.");
            return;
        }
        if !self
            .debug
            .as_ref()
            .is_some_and(DebugController::controls_allowed)
        {
            self.toasts
                .info("Simulate controls are disabled on this booth.");
            return;
        }
        if self
            .debug
            .as_ref()
            .is_some_and(DebugController::is_action_in_flight)
        {
            self.toasts
                .warn("A simulate action is already in progress.");
            return;
        }
        if let Some(debug) = self.debug.as_mut() {
            action(debug);
        }
    }

    /// Open a prompt for the rotary digit to simulate dialing.
    fn open_dial_prompt(&mut self) {
        if self.debug.is_none() {
            self.toasts.info("No booth is configured.");
            return;
        }
        if !self
            .debug
            .as_ref()
            .is_some_and(DebugController::controls_allowed)
        {
            self.toasts
                .info("Simulate controls are disabled on this booth.");
            return;
        }
        self.modal = Some(Modal::prompt(
            "Simulate dial",
            "Rotary digit (0-9)",
            Intent::SimulateDial,
        ));
    }

    /// Parse a single rotary digit and simulate dialing it.
    fn simulate_dial(&mut self, input: &str) {
        let digit = match input.trim().parse::<u8>() {
            Ok(digit) if digit <= 9 => digit,
            _ => {
                self.toasts.warn("Enter a single digit 0-9.");
                return;
            }
        };
        self.debug_simulate(move |debug| debug.simulate_pulse_digit(digit));
    }

    /// Surface completed booth simulate outcomes as toasts.
    fn drain_debug_actions(&mut self) {
        let Some(debug) = self.debug.as_mut() else {
            return;
        };
        let outcomes = debug.drain_actions();
        for outcome in outcomes {
            if outcome.ok {
                self.toasts.info(outcome.message);
            } else {
                self.toasts.error(outcome.message);
            }
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
            Screen::SystemHealth => {
                if let Some(health) = self.system_health.as_mut() {
                    health.refresh();
                }
            }
            Screen::Debug => {
                if let Some(debug) = self.debug.as_mut() {
                    debug.refresh();
                }
            }
            Screen::Tokens => self.tokens.refresh(),
            Screen::Settings | Screen::About => {}
        }
    }

    /// Advance the selection on the active list screen, if any.
    fn select_next_active(&mut self) {
        match self.screen {
            Screen::Messages => self.messages.select_next(),
            Screen::Questions => self.questions.select_next(),
            Screen::Sessions => self.sessions.select_next(),
            Screen::Events => self.events.select_next(),
            Screen::Tokens => self.tokens.select_next(),
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
            Screen::Tokens => self.tokens.select_prev(),
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
