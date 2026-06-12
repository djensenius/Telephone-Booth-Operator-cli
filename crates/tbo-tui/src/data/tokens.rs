//! Background loading of `GET /v1/api-tokens` for the API Tokens screen, plus
//! token management actions (create, revoke) and per-token usage lookups issued
//! off the UI thread.
//!
//! Creating a token returns its plaintext secret exactly once; the controller
//! holds the freshly created token in [`TokensController::revealed`] so the UI
//! can present the secret until the operator dismisses it.

use std::future::Future;
use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::{ApiToken, ApiTokenCreated, ApiTokenUsageBucket};
use tbo_operator_client::{
    HttpTransport, OperatorClient, ReqwestTransport, TokenProvider, WriteTransport,
};

use crate::data::{ActionOutcome, Remote, SessionTokenProvider};

/// Look-back window, in days, for the per-token usage lookup.
const USAGE_DAYS: u32 = 30;

/// The outcome of a completed token management action, carrying the created
/// token when one was minted so the controller can reveal its secret.
enum ActionResult {
    /// A token was created; the boxed value carries the one-time secret.
    Created(Box<ApiTokenCreated>),
    /// A token was revoked.
    Revoked,
    /// The action failed with a human-readable message.
    Failed(String),
}

/// Loads the API-token list off the UI thread, tracks the selected row, and
/// issues create/revoke/usage requests.
///
/// Like the other management controllers this loads once when the screen is
/// first focused and thereafter only on demand; a successful create or revoke
/// schedules a reload so the list reflects the change.
pub struct TokensController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    state: Remote<Vec<ApiToken>>,
    selected: usize,
    rx: Option<UnboundedReceiver<std::result::Result<Vec<ApiToken>, String>>>,
    in_flight: bool,
    loaded: bool,
    action_rx: Option<UnboundedReceiver<ActionResult>>,
    action_in_flight: bool,
    revealed: Option<ApiTokenCreated>,
    usage: Remote<Vec<ApiTokenUsageBucket>>,
    usage_for: Option<String>,
    usage_rx: Option<UnboundedReceiver<std::result::Result<Vec<ApiTokenUsageBucket>, String>>>,
    usage_in_flight: bool,
}

impl<T, A> TokensController<T, A>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Build a controller over the given operator client.
    pub fn new(client: OperatorClient<T, A>) -> Self {
        Self {
            client,
            state: Remote::Idle,
            selected: 0,
            rx: None,
            in_flight: false,
            loaded: false,
            action_rx: None,
            action_in_flight: false,
            revealed: None,
            usage: Remote::Idle,
            usage_for: None,
            usage_rx: None,
            usage_in_flight: false,
        }
    }

    /// The current load state.
    #[must_use]
    pub fn state(&self) -> &Remote<Vec<ApiToken>> {
        &self.state
    }

    /// The index of the selected row.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected token, when the list is loaded and non-empty.
    #[must_use]
    pub fn selected_token(&self) -> Option<&ApiToken> {
        match &self.state {
            Remote::Ready { value, .. } => value.get(self.selected),
            _ => None,
        }
    }

    /// The freshly created token whose plaintext secret is still being shown,
    /// if any.
    #[must_use]
    pub fn revealed(&self) -> Option<&ApiTokenCreated> {
        self.revealed.as_ref()
    }

    /// Dismiss the revealed plaintext secret, returning whether one was shown.
    pub fn dismiss_revealed(&mut self) -> bool {
        self.revealed.take().is_some()
    }

    /// The usage state for the selected token, when it matches the most recent
    /// usage lookup.
    #[must_use]
    pub fn usage(&self) -> Option<&Remote<Vec<ApiTokenUsageBucket>>> {
        match (self.usage_for.as_deref(), self.selected_token()) {
            (Some(loaded), Some(token)) if loaded == token.id => Some(&self.usage),
            _ => None,
        }
    }

    /// Whether a load is currently in flight.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.in_flight
    }

    /// Trigger a load unless one is already in flight.
    pub fn refresh(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        if matches!(self.state, Remote::Idle | Remote::Failed { .. }) {
            self.state = Remote::Loading;
        }
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client.api_tokens().await.map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Move the selection to the next row, if any.
    pub fn select_next(&mut self) {
        if let Remote::Ready { value, .. } = &self.state
            && self.selected + 1 < value.len()
        {
            self.selected += 1;
        }
    }

    /// Move the selection to the previous row, if any.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Load usage buckets for the selected token unless a lookup is in flight.
    pub fn load_usage_selected(&mut self) {
        if self.usage_in_flight {
            return;
        }
        let Some(id) = self.selected_id() else { return };
        self.usage_in_flight = true;
        self.usage = Remote::Loading;
        self.usage_for = Some(id.clone());
        let (tx, rx) = unbounded_channel();
        self.usage_rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client
                .api_token_usage(&id, Some(USAGE_DAYS))
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Apply any completed list or usage loads (non-blocking). Called each tick.
    pub fn drain(&mut self) {
        self.drain_list();
        self.drain_usage();
    }

    /// Apply any completed list load.
    fn drain_list(&mut self) {
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    /// Apply any completed usage load.
    fn drain_usage(&mut self) {
        let Some(rx) = self.usage_rx.as_mut() else {
            return;
        };
        match rx.try_recv() {
            Ok(result) => self.apply_usage(result),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.usage_rx = None;
                self.usage_in_flight = false;
            }
        }
    }

    /// Advance the controller: apply results, then perform the initial load the
    /// first time the screen is focused.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused && !self.loaded && !self.in_flight {
            self.refresh();
        }
    }

    /// Apply a single list result to the visible state.
    fn apply(&mut self, result: std::result::Result<Vec<ApiToken>, String>) {
        self.in_flight = false;
        self.loaded = true;
        self.rx = None;
        match result {
            Ok(items) => {
                self.selected = self.selected.min(items.len().saturating_sub(1));
                self.state = Remote::Ready {
                    value: items,
                    fetched_at: Instant::now(),
                };
            }
            Err(error) => {
                self.state = Remote::Failed {
                    error,
                    at: Instant::now(),
                };
            }
        }
    }

    /// Apply a single usage result to the usage state.
    fn apply_usage(&mut self, result: std::result::Result<Vec<ApiTokenUsageBucket>, String>) {
        self.usage_in_flight = false;
        self.usage_rx = None;
        self.usage = match result {
            Ok(buckets) => Remote::Ready {
                value: buckets,
                fetched_at: Instant::now(),
            },
            Err(error) => Remote::Failed {
                error,
                at: Instant::now(),
            },
        };
    }

    /// Whether a write action (create or revoke) is in flight.
    #[must_use]
    pub fn is_action_in_flight(&self) -> bool {
        self.action_in_flight
    }

    /// Drain any completed write-action outcome (called each tick). A
    /// successful create reveals the new secret; a successful create or revoke
    /// schedules a list reload.
    pub fn drain_actions(&mut self) -> Vec<ActionOutcome> {
        let Some(rx) = self.action_rx.as_mut() else {
            return Vec::new();
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return Vec::new(),
            Err(TryRecvError::Disconnected) => {
                self.action_in_flight = false;
                self.action_rx = None;
                return Vec::new();
            }
        };
        vec![self.apply_action(result)]
    }

    /// Apply one completed action result, updating reveal/reload state and
    /// producing the toast-worthy outcome.
    fn apply_action(&mut self, result: ActionResult) -> ActionOutcome {
        self.action_in_flight = false;
        self.action_rx = None;
        match result {
            ActionResult::Created(token) => {
                let message = format!("Token \"{}\" created — copy the secret now.", token.name);
                self.revealed = Some(*token);
                self.loaded = false;
                ActionOutcome { message, ok: true }
            }
            ActionResult::Revoked => {
                self.loaded = false;
                ActionOutcome {
                    message: "Token revoked.".to_owned(),
                    ok: true,
                }
            }
            ActionResult::Failed(message) => ActionOutcome { message, ok: false },
        }
    }

    /// The selected token's id, when the list is loaded and non-empty.
    fn selected_id(&self) -> Option<String> {
        self.selected_token().map(|token| token.id.clone())
    }

    /// Await and apply the next pending list result (test helper).
    #[cfg(test)]
    async fn recv_once(&mut self) {
        if let Some(rx) = self.rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            self.apply(result);
        }
    }

    /// Await and apply the next pending usage result (test helper).
    #[cfg(test)]
    async fn recv_usage_once(&mut self) {
        if let Some(rx) = self.usage_rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            self.apply_usage(result);
        }
    }

    /// Await the next completed action and apply it (test helper).
    #[cfg(test)]
    async fn recv_action_once(&mut self) -> Vec<ActionOutcome> {
        let Some(rx) = self.action_rx.as_mut() else {
            return Vec::new();
        };
        let Some(result) = rx.recv().await else {
            return Vec::new();
        };
        vec![self.apply_action(result)]
    }
}

impl<T, A> TokensController<T, A>
where
    T: WriteTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Create a token named `name`, never expiring (`POST /v1/api-tokens`).
    pub fn create_token(&mut self, name: String) {
        let client = self.client.clone();
        self.run_action(async move {
            match client.create_api_token(name, None).await {
                Ok(created) => ActionResult::Created(Box::new(created)),
                Err(err) => ActionResult::Failed(format!("Create failed: {err}")),
            }
        });
    }

    /// Revoke the selected token (`DELETE /v1/api-tokens/{id}`).
    pub fn revoke_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action(async move {
            match client.revoke_api_token(&id).await {
                Ok(()) => ActionResult::Revoked,
                Err(err) => ActionResult::Failed(format!("Revoke failed: {err}")),
            }
        });
    }

    /// Spawn a write action, recording its result for the next `drain_actions`.
    /// A no-op when another action is already in flight.
    fn run_action<F>(&mut self, future: F)
    where
        F: Future<Output = ActionResult> + Send + 'static,
    {
        if self.action_in_flight {
            return;
        }
        self.action_in_flight = true;
        let (tx, rx) = unbounded_channel();
        self.action_rx = Some(rx);
        tokio::spawn(async move {
            let _ = tx.send(future.await);
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use tbo_operator_client::{
        HttpResponse, HttpTransport, Result, StaticTokenProvider, WriteTransport,
    };

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        list: Arc<Mutex<HttpResponse>>,
        usage: Arc<Mutex<HttpResponse>>,
        write: Arc<Mutex<HttpResponse>>,
    }

    impl FakeTransport {
        fn new(list_status: u16, list_body: &str) -> Self {
            Self {
                list: Arc::new(Mutex::new(HttpResponse {
                    status: list_status,
                    body: list_body.to_owned(),
                })),
                usage: Arc::new(Mutex::new(HttpResponse {
                    status: 200,
                    body: "[]".to_owned(),
                })),
                write: Arc::new(Mutex::new(HttpResponse {
                    status: 200,
                    body: String::new(),
                })),
            }
        }

        fn with_usage(self, status: u16, body: &str) -> Self {
            *self.usage.lock().unwrap() = HttpResponse {
                status,
                body: body.to_owned(),
            };
            self
        }

        fn with_write(self, status: u16, body: &str) -> Self {
            *self.write.lock().unwrap() = HttpResponse {
                status,
                body: body.to_owned(),
            };
            self
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            let slot = if path.contains("/usage") {
                &self.usage
            } else {
                &self.list
            };
            Ok(slot.lock().unwrap().clone())
        }
    }

    impl WriteTransport for FakeTransport {
        async fn post(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
            _json_body: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(self.write.lock().unwrap().clone())
        }

        async fn delete(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(self.write.lock().unwrap().clone())
        }

        async fn put_bytes(
            &self,
            _url: &str,
            _content_type: &str,
            _body: Vec<u8>,
        ) -> Result<HttpResponse> {
            Ok(self.write.lock().unwrap().clone())
        }
    }

    fn controller(
        transport: FakeTransport,
    ) -> TokensController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::new("token"));
        TokensController::new(client)
    }

    fn token_json(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","name":"Token {id}","last4":"ab{id}","createdAt":"2026-01-01T00:00:00Z","expiresAt":null,"lastUsedAt":null,"revokedAt":null}}"#
        )
    }

    fn created_json(id: &str, name: &str, plaintext: &str) -> String {
        format!(
            r#"{{"id":"{id}","name":"{name}","last4":"wxyz","createdAt":"2026-01-01T00:00:00Z","expiresAt":null,"plaintext":"{plaintext}"}}"#
        )
    }

    #[tokio::test]
    async fn refresh_loads_tokens_into_ready() {
        let body = format!("[{},{}]", token_json("a"), token_json("b"));
        let mut controller = controller(FakeTransport::new(200, &body));

        controller.refresh();
        controller.recv_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 2),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert_eq!(
            controller.selected_token().map(|t| t.id.as_str()),
            Some("a")
        );
    }

    #[tokio::test]
    async fn selection_moves_and_clamps() {
        let body = format!("[{},{}]", token_json("a"), token_json("b"));
        let mut controller = controller(FakeTransport::new(200, &body));
        controller.refresh();
        controller.recv_once().await;

        controller.select_prev();
        assert_eq!(controller.selected_index(), 0);
        controller.select_next();
        assert_eq!(controller.selected_index(), 1);
        controller.select_next();
        assert_eq!(controller.selected_index(), 1);
    }

    #[tokio::test]
    async fn failed_load_becomes_failed_state() {
        let mut controller = controller(FakeTransport::new(401, ""));
        controller.refresh();
        controller.recv_once().await;
        assert!(matches!(controller.state(), Remote::Failed { .. }));
        assert!(!controller.is_refreshing());
    }

    #[tokio::test]
    async fn create_token_reveals_the_secret() {
        let transport =
            FakeTransport::new(200, "[]").with_write(201, &created_json("t1", "ci", "tbk_secret"));
        let mut controller = controller(transport);

        controller.create_token("ci".to_owned());
        assert!(controller.is_action_in_flight());
        let outcomes = controller.recv_action_once().await;

        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].ok);
        assert!(outcomes[0].message.contains("created"));
        assert_eq!(
            controller.revealed().map(|t| t.plaintext.as_str()),
            Some("tbk_secret")
        );
        assert!(
            !controller.loaded,
            "a successful create must schedule a reload"
        );
    }

    #[tokio::test]
    async fn dismiss_revealed_clears_the_secret() {
        let transport =
            FakeTransport::new(200, "[]").with_write(201, &created_json("t1", "ci", "tbk_secret"));
        let mut controller = controller(transport);
        controller.create_token("ci".to_owned());
        controller.recv_action_once().await;

        assert!(controller.dismiss_revealed());
        assert!(controller.revealed().is_none());
        assert!(!controller.dismiss_revealed(), "nothing left to dismiss");
    }

    #[tokio::test]
    async fn revoke_selected_reports_success() {
        let list = format!("[{}]", token_json("a"));
        let transport = FakeTransport::new(200, &list).with_write(204, "");
        let mut controller = controller(transport);
        controller.refresh();
        controller.recv_once().await;

        controller.revoke_selected();
        let outcomes = controller.recv_action_once().await;
        assert!(outcomes[0].ok);
        assert!(outcomes[0].message.contains("revoked"));
    }

    #[tokio::test]
    async fn revoke_is_a_noop_without_selection() {
        let mut controller = controller(FakeTransport::new(200, "[]"));
        controller.refresh();
        controller.recv_once().await;

        controller.revoke_selected();
        assert!(!controller.is_action_in_flight());
    }

    #[tokio::test]
    async fn create_failure_surfaces_as_an_error() {
        let transport = FakeTransport::new(200, "[]").with_write(401, "");
        let mut controller = controller(transport);

        controller.create_token("ci".to_owned());
        let outcomes = controller.recv_action_once().await;
        assert!(!outcomes[0].ok);
        assert!(outcomes[0].message.contains("Create failed"));
        assert!(controller.revealed().is_none());
    }

    #[tokio::test]
    async fn usage_loads_for_the_selected_token() {
        let list = format!("[{}]", token_json("a"));
        let transport =
            FakeTransport::new(200, &list).with_usage(200, r#"[{"date":"2026-01-01","count":4}]"#);
        let mut controller = controller(transport);
        controller.refresh();
        controller.recv_once().await;

        controller.load_usage_selected();
        controller.recv_usage_once().await;

        match controller.usage() {
            Some(Remote::Ready { value, .. }) => {
                assert_eq!(value.len(), 1);
                assert_eq!(value[0].count, 4);
            }
            other => panic!("expected Ready usage, got {other:?}"),
        }
    }
}
