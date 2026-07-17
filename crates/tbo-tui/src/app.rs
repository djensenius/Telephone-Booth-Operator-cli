//! Application state and the main event loop.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tbo_core::config::Config;
use tbo_operator_client::OperatorClient;
use tokio::time::Duration;

use crate::auth::{AuthController, AuthPhase};
use crate::data::{
    DebugController, EventsController, IdentityController, MessagesController, PlaybackController,
    QuestionsController, SessionTokenProvider, SessionsController, SharedSession, StatsController,
    StatusController, SystemController, SystemHealthController, TokensController,
};
use crate::event::{AppEvent, EventLoop};
use crate::tui::Tui;
use crate::ui;
use crate::ui::icons::Icons;
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
    icons: Icons,
    screen: Screen,
    toasts: Toasts,
    auth: AuthController,
    identity: IdentityController,
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
    show_help: bool,
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
        let icons = Icons::new(config.ui.nerd_fonts);
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
        let identity = IdentityController::new(api.clone());
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
            icons,
            screen: Screen::Status,
            toasts,
            auth,
            identity,
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
            show_help: false,
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

    /// Build the shared session manager via [`crate::data::build_shared_session`],
    /// surfacing any fallback warning as a toast.
    fn build_session(config: &Config, toasts: &mut Toasts) -> Result<SharedSession> {
        let (session, warning) = crate::data::build_shared_session(config)?;
        if let Some(warning) = warning {
            toasts.warn(warning);
        }
        Ok(session)
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

    /// The resolved Nerd Font glyph set for chrome rendering.
    #[must_use]
    pub fn icons(&self) -> Icons {
        self.icons
    }

    /// Whether the `?` help overlay is currently shown.
    #[must_use]
    pub fn show_help(&self) -> bool {
        self.show_help
    }

    /// The currently focused screen.
    #[must_use]
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// Whether the signed-in operator is a known administrator. Used by the
    /// UI to dim/lock admin-only screens in the palette and header.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.identity.is_admin()
    }

    /// The next screen the current operator may open, skipping admin-only
    /// screens (API tokens, debug) when they are not an administrator.
    #[must_use]
    pub fn next_screen(&self) -> Screen {
        self.reachable_screen(true)
    }

    /// The previous reachable screen, mirroring [`Self::next_screen`].
    #[must_use]
    pub fn prev_screen(&self) -> Screen {
        self.reachable_screen(false)
    }

    /// Step forward/backward from the current screen, skipping screens the
    /// operator may not open. Falls back to the current screen if every other
    /// screen is gated (which cannot happen in practice).
    fn reachable_screen(&self, forward: bool) -> Screen {
        let mut candidate = if forward {
            self.screen.next()
        } else {
            self.screen.prev()
        };
        for _ in 0..Screen::count() {
            if !candidate.is_admin_only() || self.identity.is_admin() {
                return candidate;
            }
            candidate = if forward {
                candidate.next()
            } else {
                candidate.prev()
            };
        }
        self.screen
    }

    /// Whether the operator may open `screen`, nudging with a toast when they
    /// may not (non-admin, or permissions not yet loaded). Mirrors
    /// [`Self::require_question_admin`].
    fn require_screen_admin(&mut self, screen: Screen) -> bool {
        if !screen.is_admin_only() || self.identity.is_admin() {
            return true;
        }
        if self.identity.is_known() {
            self.toasts.warn(format!(
                "{} requires an administrator account.",
                screen.title()
            ));
        } else {
            self.toasts
                .info("Checking your permissions… try again in a moment.");
        }
        false
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
                    let signed_in = matches!(self.auth.phase(), AuthPhase::SignedIn { .. });
                    self.identity.tick(signed_in);
                    if self.identity.take_revoked() {
                        self.toasts.error(
                            "Your operator account is no longer valid; you have been signed out.",
                        );
                        self.auth.sign_out(&mut self.toasts);
                        self.identity.reset();
                    }
                    // If the operator's tier drops (e.g. revalidation returns
                    // a non-admin) while an admin-only screen is focused, bounce
                    // them back to Status so gated content never lingers.
                    if self.screen.is_admin_only()
                        && self.identity.is_known()
                        && !self.identity.is_admin()
                    {
                        self.screen = Screen::Status;
                    }
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
        // The help overlay likewise captures input (and offers login/logout).
        if self.show_help {
            self.handle_help_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('g' | 'G') if self.screen != Screen::Messages => self.show_help = true,
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
            KeyCode::Tab | KeyCode::Right => self.screen = self.next_screen(),
            KeyCode::BackTab | KeyCode::Left => self.screen = self.prev_screen(),
            KeyCode::Char('u' | 'U') if self.screen == Screen::Settings => {
                self.open_operator_url_prompt();
            }
            KeyCode::Char('b' | 'B') if self.screen == Screen::Settings => {
                self.open_booth_url_prompt();
            }
            KeyCode::Char('k' | 'K') if self.screen == Screen::Settings => {
                self.open_booth_token_prompt();
            }
            KeyCode::Char('p' | 'P') if self.screen == Screen::Settings => {
                self.open_poll_interval_prompt();
            }
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
            KeyCode::Char('l' | 'L') => self.begin_login(),
            KeyCode::Char('o' | 'O') if self.screen == Screen::Settings => {
                self.auth.sign_out(&mut self.toasts);
            }
            KeyCode::Char('t' | 'T') if self.screen == Screen::Settings => self.cycle_theme(),
            KeyCode::Char(c) if c.is_ascii_digit() => self.jump_to_nav_key(c),
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
            Intent::EditOperatorBaseUrl => {
                let input = modal.input().to_owned();
                self.update_operator_base_url(&input);
            }
            Intent::EditBoothDebugUrl => {
                let input = modal.input().to_owned();
                self.update_first_booth_url(&input);
            }
            Intent::EditBoothDebugToken => {
                let input = modal.input().to_owned();
                self.update_first_booth_token(&input);
            }
            Intent::EditPollInterval => {
                let input = modal.input().to_owned();
                self.update_poll_interval(&input);
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

    /// Whether the signed-in operator may manage questions, nudging with a
    /// toast when they may not (non-admin, or permissions not yet loaded).
    fn require_question_admin(&mut self) -> bool {
        if self.identity.is_admin() {
            return true;
        }
        if self.identity.is_known() {
            self.toasts
                .warn("Managing questions requires an administrator account.");
        } else {
            self.toasts
                .info("Checking your permissions… try again in a moment.");
        }
        false
    }

    /// Run a question write action, guarding against overlapping requests and
    /// nudging when there is nothing selected.
    fn question_action(&mut self, action: impl FnOnce(&mut QuestionsController)) {
        if !self.require_question_admin() {
            return;
        }
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
        if !self.require_question_admin() {
            return;
        }
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
        if !self.require_question_admin() {
            return;
        }
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
        match self.persist_config_and_secrets() {
            Ok(()) => self.toasts.info(format!("Theme: {next}")),
            Err(err) => self.toasts.warn(format!(
                "Theme set to {next}, but saving config failed: {err}"
            )),
        }
    }

    /// Open a prompt to edit the operator API URL.
    fn open_operator_url_prompt(&mut self) {
        self.modal = Some(Modal::prompt_with_input(
            "Edit operator API",
            "Base URL",
            Intent::EditOperatorBaseUrl,
            self.config.operator.base_url.clone(),
        ));
    }

    /// Open a prompt to edit the first configured booth's debug URL.
    fn open_booth_url_prompt(&mut self) {
        let Some(booth) = self.config.booths.first() else {
            self.toasts.info("No booth is configured.");
            return;
        };
        self.modal = Some(Modal::prompt_with_input(
            "Edit booth debug URL",
            format!(
                "Debug URL for {}",
                booth.name.as_deref().unwrap_or(&booth.id)
            ),
            Intent::EditBoothDebugUrl,
            booth.debug_base_url.clone(),
        ));
    }

    /// Open a prompt to replace the first configured booth's debug token.
    fn open_booth_token_prompt(&mut self) {
        let Some(booth) = self.config.booths.first() else {
            self.toasts.info("No booth is configured.");
            return;
        };
        self.modal = Some(Modal::prompt(
            "Edit booth debug token",
            format!(
                "Paste new token for {}",
                booth.name.as_deref().unwrap_or(&booth.id)
            ),
            Intent::EditBoothDebugToken,
        ));
    }

    /// Open a prompt to edit the displayed poll interval preference.
    fn open_poll_interval_prompt(&mut self) {
        self.modal = Some(Modal::prompt_with_input(
            "Edit poll interval",
            "Milliseconds",
            Intent::EditPollInterval,
            self.config.ui.poll_interval_ms.to_string(),
        ));
    }

    /// Persist a changed operator API base URL.
    fn update_operator_base_url(&mut self, input: &str) {
        let value = input.trim();
        if !is_http_url(value) {
            self.toasts.warn("Enter an http:// or https:// URL.");
            return;
        }
        self.config.operator.base_url = value.to_owned();
        match self.persist_config_and_secrets() {
            Ok(()) => self
                .toasts
                .info("Operator API URL saved. Restart to rebuild API clients."),
            Err(err) => self.toasts.error(format!("Saving config failed: {err}")),
        }
    }

    /// Persist and apply the first configured booth's debug URL.
    fn update_first_booth_url(&mut self, input: &str) {
        let value = input.trim();
        if !is_http_url(value) {
            self.toasts.warn("Enter an http:// or https:// URL.");
            return;
        }
        let Some(booth) = self.config.booths.first_mut() else {
            self.toasts.info("No booth is configured.");
            return;
        };
        booth.debug_base_url = value.to_owned();
        match self.persist_config_and_secrets() {
            Ok(()) => {
                self.rebuild_booth_controllers();
                self.toasts.info("Booth debug URL saved.");
            }
            Err(err) => self.toasts.error(format!("Saving config failed: {err}")),
        }
    }

    /// Persist and apply the first configured booth's debug token.
    fn update_first_booth_token(&mut self, input: &str) {
        let value = input.trim();
        if value.is_empty() {
            self.toasts.warn("Paste a non-empty debug token.");
            return;
        }
        let Some(booth) = self.config.booths.first_mut() else {
            self.toasts.info("No booth is configured.");
            return;
        };
        booth.debug_token = Some(value.to_owned());
        match self.persist_config_and_secrets() {
            Ok(()) => {
                self.rebuild_booth_controllers();
                self.toasts.info("Booth debug token saved.");
            }
            Err(err) => self.toasts.error(format!("Saving config failed: {err}")),
        }
    }

    /// Persist the UI poll interval preference.
    fn update_poll_interval(&mut self, input: &str) {
        let Ok(value) = input.trim().parse::<u64>() else {
            self.toasts.warn("Enter a whole number of milliseconds.");
            return;
        };
        if value < 250 {
            self.toasts.warn("Poll interval must be at least 250 ms.");
            return;
        }
        self.config.ui.poll_interval_ms = value;
        match self.persist_config_and_secrets() {
            Ok(()) => self
                .toasts
                .info(format!("Poll interval saved: {value} ms.")),
            Err(err) => self.toasts.error(format!("Saving config failed: {err}")),
        }
    }

    /// Save config and secrets without leaking debug tokens into config.toml.
    fn persist_config_and_secrets(&self) -> std::result::Result<(), String> {
        let mut shareable = self.config.clone();
        let secrets = shareable.take_secrets();
        shareable
            .save_to(&self.config_path)
            .map_err(|err| err.to_string())?;
        secrets.save().map_err(|err| err.to_string())
    }

    /// Rebuild booth-direct controllers after Settings changes.
    fn rebuild_booth_controllers(&mut self) {
        self.system_health = Self::build_system_health(&self.config, &mut self.toasts);
        self.debug = Self::build_debug(&self.config, &mut self.toasts);
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

    /// Route a key to the `?` help overlay: close it, or run login/logout. The
    /// overlay stays open during login so the device code stays visible.
    fn handle_help_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?' | 'q') => self.show_help = false,
            KeyCode::Tab | KeyCode::Right => self.screen = self.next_screen(),
            KeyCode::BackTab | KeyCode::Left => self.screen = self.prev_screen(),
            KeyCode::Char(c) if Self::is_screen_palette_key(c) => {
                self.jump_to_nav_key(c);
                self.show_help = false;
            }
            KeyCode::Char('l' | 'L') => self.begin_login(),
            KeyCode::Char('o' | 'O') => self.auth.sign_out(&mut self.toasts),
            _ => {}
        }
    }

    /// Whether `key` is used by the screen palette.
    fn is_screen_palette_key(key: char) -> bool {
        Screen::from_nav_key(key).is_some()
    }

    /// Jump to the screen addressed by a palette key, if it exists.
    fn jump_to_nav_key(&mut self, key: char) {
        if let Some(screen) = Screen::from_nav_key(key)
            && self.require_screen_admin(screen)
        {
            self.screen = screen;
        }
    }
}

/// Minimal URL guard for editable config fields.
fn is_http_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}
