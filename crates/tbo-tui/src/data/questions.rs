//! Background loading of `GET /v1/questions` for the Questions screen, plus
//! operator management actions (activate/deactivate, archive, and create via
//! the audio-upload SAS flow) issued off the UI thread.

use std::future::Future;
use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::Question;
use tbo_operator_client::{
    HttpTransport, OperatorClient, ReqwestTransport, TokenProvider, WriteTransport,
};

use crate::data::{ActionOutcome, Remote, SessionTokenProvider};

/// How many questions to request per load.
const PAGE_LIMIT: u32 = 50;

/// Loads the question list off the UI thread and tracks the selected row.
///
/// Like the messages poller this loads once when the screen is first focused
/// and thereafter only on demand, so a reload never disrupts the selection.
pub struct QuestionsController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    state: Remote<Vec<Question>>,
    selected: usize,
    rx: Option<UnboundedReceiver<std::result::Result<Vec<Question>, String>>>,
    in_flight: bool,
    loaded: bool,
    action_rx: Option<UnboundedReceiver<ActionOutcome>>,
    action_in_flight: bool,
}

impl<T, A> QuestionsController<T, A>
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
    pub fn state(&self) -> &Remote<Vec<Question>> {
        &self.state
    }

    /// The index of the selected row.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected question, when the list is loaded and non-empty.
    #[must_use]
    pub fn selected_question(&self) -> Option<&Question> {
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
                .questions(None, None, Some(PAGE_LIMIT))
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
    fn apply(&mut self, result: std::result::Result<Vec<Question>, String>) {
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

    /// Whether a write action (activate, archive, create, …) is in flight.
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

impl<T, A> QuestionsController<T, A>
where
    T: WriteTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Publish the selected question (`POST /activate`).
    pub fn activate_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Question activated.".to_owned(), async move {
            client
                .activate_question(&id)
                .await
                .map(|_| ())
                .map_err(|err| format!("Activate failed: {err}"))
        });
    }

    /// Return the selected question to `draft` (`POST /deactivate`).
    pub fn deactivate_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Question deactivated.".to_owned(), async move {
            client
                .deactivate_question(&id)
                .await
                .map(|_| ())
                .map_err(|err| format!("Deactivate failed: {err}"))
        });
    }

    /// Archive (retire) the selected question (`DELETE /v1/questions/{id}`).
    pub fn archive_selected(&mut self) {
        let Some(id) = self.selected_id() else { return };
        let client = self.client.clone();
        self.run_action("Question archived.".to_owned(), async move {
            client
                .archive_question(&id)
                .await
                .map_err(|err| format!("Archive failed: {err}"))
        });
    }

    /// Create a question from `prompt` text and a FLAC file at `audio_path`.
    ///
    /// Reads the file off the UI thread, then runs the upload-and-create flow.
    pub fn create_question(&mut self, prompt: String, audio_path: String) {
        let client = self.client.clone();
        self.run_action("Question created.".to_owned(), async move {
            let audio = tokio::fs::read(&audio_path)
                .await
                .map_err(|err| format!("Could not read {audio_path}: {err}"))?;
            client
                .create_question_with_audio(prompt, audio, None)
                .await
                .map(|_| ())
                .map_err(|err| format!("Create failed: {err}"))
        });
    }

    /// The selected question's id, when the list is loaded and non-empty.
    fn selected_id(&self) -> Option<String> {
        self.selected_question().map(|question| question.id.clone())
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
    }

    fn controller(
        status: u16,
        body: &str,
    ) -> QuestionsController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(status, body),
            StaticTokenProvider::new("token"),
        );
        QuestionsController::new(client)
    }

    fn controller_with_write(
        list_body: &str,
        write_status: u16,
        write_body: &str,
    ) -> QuestionsController<FakeTransport, StaticTokenProvider> {
        let transport = FakeTransport::new(200, list_body).with_write(write_status, write_body);
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::new("token"));
        QuestionsController::new(client)
    }

    fn question_json(id: &str, status: &str) -> String {
        format!(
            r#"{{"id":"{id}","prompt":"Prompt {id}","status":"{status}","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"https://example/{id}.flac","sha256":"{id}","durationMs":1000}}}}"#
        )
    }

    #[tokio::test]
    async fn refresh_loads_questions_into_ready() {
        let body = format!(
            r#"{{"items":[{},{}]}}"#,
            question_json("a", "active"),
            question_json("b", "archived")
        );
        let mut controller = controller(200, &body);

        controller.refresh();
        controller.recv_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 2),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert_eq!(
            controller.selected_question().map(|q| q.id.as_str()),
            Some("a")
        );
    }

    #[tokio::test]
    async fn selection_moves_and_clamps() {
        let body = format!(
            r#"{{"items":[{},{}]}}"#,
            question_json("a", "active"),
            question_json("b", "draft")
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
            controller.selected_question().map(|q| q.id.as_str()),
            Some("b")
        );
    }

    #[tokio::test]
    async fn reload_clamps_selection_to_shorter_list() {
        let two = format!(
            r#"{{"items":[{},{}]}}"#,
            question_json("a", "active"),
            question_json("b", "active")
        );
        let mut controller = controller(200, &two);
        controller.refresh();
        controller.recv_once().await;
        controller.select_next();
        assert_eq!(controller.selected_index(), 1);

        // A subsequent load returns no items; selection must clamp to zero.
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
    async fn activate_selected_reports_success() {
        let list = format!(r#"{{"items":[{}]}}"#, question_json("a", "draft"));
        let mut controller = controller_with_write(&list, 200, &question_json("a", "active"));
        controller.refresh();
        controller.recv_once().await;

        controller.activate_selected();
        assert!(controller.is_action_in_flight());
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(outcome.ok, "activate should succeed");
        assert!(outcome.message.contains("activated"));
    }

    #[tokio::test]
    async fn archive_selected_reports_success_on_no_content() {
        let list = format!(r#"{{"items":[{}]}}"#, question_json("a", "active"));
        let mut controller = controller_with_write(&list, 204, "");
        controller.refresh();
        controller.recv_once().await;

        controller.archive_selected();
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(outcome.ok);
        assert!(outcome.message.contains("archived"));
    }

    #[tokio::test]
    async fn action_reports_failure_on_not_found() {
        let list = format!(r#"{{"items":[{}]}}"#, question_json("a", "draft"));
        let mut controller = controller_with_write(&list, 404, r#"{"error":"not_found"}"#);
        controller.refresh();
        controller.recv_once().await;

        controller.deactivate_selected();
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(!outcome.ok, "a 404 must surface as a failure");
        assert!(outcome.message.contains("Deactivate failed"));
    }

    #[tokio::test]
    async fn action_is_a_noop_without_selection() {
        let mut controller = controller(200, r#"{"items":[]}"#);
        controller.refresh();
        controller.recv_once().await;

        controller.activate_selected();
        assert!(
            !controller.is_action_in_flight(),
            "no selection means no action is spawned"
        );
    }

    #[tokio::test]
    async fn create_question_reports_failure_for_a_missing_file() {
        let mut controller = controller_with_write(r#"{"items":[]}"#, 201, "");

        controller.create_question(
            "A new prompt?".to_owned(),
            "/nonexistent/path/to/audio.flac".to_owned(),
        );
        assert!(controller.is_action_in_flight());
        let outcome = controller.recv_action_once().await.expect("an outcome");
        assert!(!outcome.ok, "an unreadable file must fail the action");
        assert!(outcome.message.contains("Could not read"));
    }
}
