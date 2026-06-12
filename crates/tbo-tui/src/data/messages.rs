//! Background loading of `GET /v1/messages` for the Messages screen, plus
//! operator moderation actions (approve/reject, translation, re-transcribe,
//! re-moderate, delete) issued off the UI thread.

use std::future::Future;
use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::{Message, MessageDecisionKind};
use tbo_operator_client::{
    HttpTransport, OperatorClient, ReqwestTransport, TokenProvider, WriteTransport,
};

use crate::data::{ActionOutcome, Remote, SessionTokenProvider};

/// How many messages to request per load.
const PAGE_LIMIT: u32 = 50;

/// Loads the message list off the UI thread and tracks the selected row.
///
/// Unlike the status poller this does not auto-refresh on a timer (a reload
/// while browsing would disrupt the selection); it loads once when the screen
/// is first focused and thereafter only on demand.
pub struct MessagesController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    state: Remote<Vec<Message>>,
    selected: usize,
    rx: Option<UnboundedReceiver<std::result::Result<Vec<Message>, String>>>,
    in_flight: bool,
    loaded: bool,
    action_rx: Option<UnboundedReceiver<ActionOutcome>>,
    action_in_flight: bool,
}

impl<T, A> MessagesController<T, A>
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
        }
    }

    /// The current load state.
    #[must_use]
    pub fn state(&self) -> &Remote<Vec<Message>> {
        &self.state
    }

    /// The index of the selected row.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected message, when the list is loaded and non-empty.
    #[must_use]
    pub fn selected_message(&self) -> Option<&Message> {
        match &self.state {
            Remote::Ready { value, .. } => value.get(self.selected),
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
            let result = client
                .messages(None, None, Some(PAGE_LIMIT))
                .await
                .map(|list| list.items)
                .map_err(|err| err.to_string());
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

    /// Apply any completed load (non-blocking). Called each tick.
    pub fn drain(&mut self) {
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The load task ended without sending; recover so a later
                    // refresh can run.
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
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

    /// Apply a single load result to the visible state.
    fn apply(&mut self, result: std::result::Result<Vec<Message>, String>) {
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

    /// Whether a write action (decision, translation, delete, …) is in flight.
    #[must_use]
    pub fn is_action_in_flight(&self) -> bool {
        self.action_in_flight
    }

    /// Drain any completed write-action outcomes (called each tick). At most
    /// one outcome is pending at a time, since a new action cannot start until
    /// the previous one is drained.
    pub fn drain_actions(&mut self) -> Vec<ActionOutcome> {
        let Some(rx) = self.action_rx.as_mut() else {
            return Vec::new();
        };
        match rx.try_recv() {
            Ok(outcome) => {
                self.action_in_flight = false;
                self.action_rx = None;
                vec![outcome]
            }
            Err(TryRecvError::Empty) => Vec::new(),
            Err(TryRecvError::Disconnected) => {
                self.action_in_flight = false;
                self.action_rx = None;
                Vec::new()
            }
        }
    }

    /// Await and apply the next pending result (test helper).
    #[cfg(test)]
    async fn recv_once(&mut self) {
        if let Some(rx) = self.rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            self.apply(result);
        }
    }

    /// Await the next completed action outcome (test helper).
    #[cfg(test)]
    async fn recv_action_once(&mut self) -> Option<ActionOutcome> {
        let rx = self.action_rx.as_mut()?;
        let outcome = rx.recv().await;
        self.action_in_flight = false;
        self.action_rx = None;
        outcome
    }
}

impl<T, A> MessagesController<T, A>
where
    T: WriteTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Approve the selected message (`POST /decision`).
    pub fn approve_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Message approved.".to_owned(), async move {
            client
                .decide_message(&id, MessageDecisionKind::Approve, None)
                .await
                .map(|_| ())
                .map_err(|err| format!("Approve failed: {err}"))
        });
    }

    /// Reject the selected message (`POST /decision`).
    pub fn reject_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Message rejected.".to_owned(), async move {
            client
                .decide_message(&id, MessageDecisionKind::Reject, None)
                .await
                .map(|_| ())
                .map_err(|err| format!("Reject failed: {err}"))
        });
    }

    /// Submit an operator translation for the selected message
    /// (`POST /translation`).
    pub fn submit_translation(&mut self, text: String) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Translation submitted.".to_owned(), async move {
            client
                .submit_translation(&id, text, None)
                .await
                .map(|_| ())
                .map_err(|err| format!("Translation failed: {err}"))
        });
    }

    /// Queue a fresh transcription for the selected message
    /// (`POST /transcribe`).
    pub fn retranscribe_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Re-transcription queued.".to_owned(), async move {
            client
                .retranscribe_message(&id)
                .await
                .map(|_| ())
                .map_err(|err| format!("Re-transcribe failed: {err}"))
        });
    }

    /// Queue a fresh moderation pass for the selected message
    /// (`POST /moderate`).
    pub fn remoderate_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Re-moderation queued.".to_owned(), async move {
            client
                .remoderate_message(&id)
                .await
                .map(|_| ())
                .map_err(|err| format!("Re-moderate failed: {err}"))
        });
    }

    /// Delete the selected message (`DELETE /v1/messages/{id}`).
    pub fn delete_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Message deleted.".to_owned(), async move {
            client
                .delete_message(&id)
                .await
                .map_err(|err| format!("Delete failed: {err}"))
        });
    }

    /// The selected message's id, when the list is loaded and non-empty.
    fn selected_id(&self) -> Option<String> {
        self.selected_message().map(|message| message.id.clone())
    }

    /// Spawn a write action, recording its outcome for the next `drain_actions`.
    /// A no-op when another action is already in flight.
    fn run_action<F>(&mut self, ok_message: String, future: F)
    where
        F: Future<Output = std::result::Result<(), String>> + Send + 'static,
    {
        if self.action_in_flight {
            return;
        }
        self.action_in_flight = true;
        let (tx, rx) = unbounded_channel();
        self.action_rx = Some(rx);
        tokio::spawn(async move {
            let outcome = match future.await {
                Ok(()) => ActionOutcome {
                    message: ok_message,
                    ok: true,
                },
                Err(message) => ActionOutcome { message, ok: false },
            };
            let _ = tx.send(outcome);
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
        get_response: Arc<Mutex<HttpResponse>>,
        write_response: Arc<Mutex<HttpResponse>>,
    }

    impl FakeTransport {
        fn new(status: u16, body: &str) -> Self {
            let response = HttpResponse {
                status,
                body: body.to_owned(),
            };
            Self {
                get_response: Arc::new(Mutex::new(response.clone())),
                write_response: Arc::new(Mutex::new(response)),
            }
        }

        fn with_write(self, status: u16, body: &str) -> Self {
            *self.write_response.lock().unwrap() = HttpResponse {
                status,
                body: body.to_owned(),
            };
            self
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(self.get_response.lock().unwrap().clone())
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
            Ok(self.write_response.lock().unwrap().clone())
        }

        async fn delete(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(self.write_response.lock().unwrap().clone())
        }

        async fn put_bytes(
            &self,
            _url: &str,
            _content_type: &str,
            _body: Vec<u8>,
        ) -> Result<HttpResponse> {
            Ok(self.write_response.lock().unwrap().clone())
        }

        async fn get_bytes(&self, _url: &str) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }
    }

    fn controller(
        status: u16,
        body: &str,
    ) -> MessagesController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(status, body),
            StaticTokenProvider::new("token"),
        );
        MessagesController::new(client)
    }

    fn controller_with_write(
        list_body: &str,
        write_status: u16,
        write_body: &str,
    ) -> MessagesController<FakeTransport, StaticTokenProvider> {
        let transport = FakeTransport::new(200, list_body).with_write(write_status, write_body);
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::new("token"));
        MessagesController::new(client)
    }

    fn message_json(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","status":"pending","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"https://example/{id}.flac","sha256":"{id}","durationMs":1000}}}}"#
        )
    }

    #[tokio::test]
    async fn refresh_loads_messages_into_ready() {
        let body = format!(
            r#"{{"items":[{},{}]}}"#,
            message_json("a"),
            message_json("b")
        );
        let mut controller = controller(200, &body);

        controller.refresh();
        controller.recv_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 2),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert_eq!(
            controller.selected_message().map(|m| m.id.as_str()),
            Some("a")
        );
    }

    #[tokio::test]
    async fn selection_moves_and_clamps() {
        let body = format!(
            r#"{{"items":[{},{}]}}"#,
            message_json("a"),
            message_json("b")
        );
        let mut controller = controller(200, &body);
        controller.refresh();
        controller.recv_once().await;

        controller.select_prev(); // already at 0, stays
        assert_eq!(controller.selected_index(), 0);
        controller.select_next();
        assert_eq!(controller.selected_index(), 1);
        controller.select_next(); // at last row, stays
        assert_eq!(controller.selected_index(), 1);
        assert_eq!(
            controller.selected_message().map(|m| m.id.as_str()),
            Some("b")
        );
    }

    #[tokio::test]
    async fn reload_clamps_selection_to_shorter_list() {
        let two = format!(
            r#"{{"items":[{},{}]}}"#,
            message_json("a"),
            message_json("b")
        );
        let mut controller = controller(200, &two);
        controller.refresh();
        controller.recv_once().await;
        controller.select_next();
        assert_eq!(controller.selected_index(), 1);

        // A subsequent load returns a single item; selection must clamp.
        controller.apply(Ok(Vec::new()));
        assert_eq!(controller.selected_index(), 0);
    }

    #[tokio::test]
    async fn failed_load_becomes_failed_state() {
        let mut controller = controller(401, "");
        controller.refresh();
        controller.recv_once().await;
        assert!(matches!(controller.state(), Remote::Failed { .. }));
        assert!(!controller.is_refreshing());
    }

    #[tokio::test]
    async fn approve_selected_reports_success() {
        let list = format!(r#"{{"items":[{}]}}"#, message_json("a"));
        let mut controller = controller_with_write(&list, 200, &message_json("a"));
        controller.refresh();
        controller.recv_once().await;

        controller.approve_selected();
        assert!(controller.is_action_in_flight());
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(outcome.ok, "approve should succeed");
        assert!(outcome.message.contains("approved"));
    }

    #[tokio::test]
    async fn delete_selected_reports_success_on_no_content() {
        let list = format!(r#"{{"items":[{}]}}"#, message_json("a"));
        let mut controller = controller_with_write(&list, 204, "");
        controller.refresh();
        controller.recv_once().await;

        controller.delete_selected();
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(outcome.ok);
        assert!(outcome.message.contains("deleted"));
    }

    #[tokio::test]
    async fn action_reports_failure_on_conflict() {
        let list = format!(r#"{{"items":[{}]}}"#, message_json("a"));
        let mut controller =
            controller_with_write(&list, 409, r#"{"error":"message_not_decidable"}"#);
        controller.refresh();
        controller.recv_once().await;

        controller.reject_selected();
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(!outcome.ok, "a 409 must surface as a failure");
        assert!(outcome.message.contains("Reject failed"));
    }

    #[tokio::test]
    async fn action_is_a_noop_without_selection() {
        let mut controller = controller(200, r#"{"items":[]}"#);
        controller.refresh();
        controller.recv_once().await;

        controller.approve_selected();
        assert!(
            !controller.is_action_in_flight(),
            "no selection means no action is spawned"
        );
    }
}
