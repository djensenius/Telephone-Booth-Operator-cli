//! Background loading of `GET /v1/questions` for the Questions screen.

use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::Question;
use tbo_operator_client::{HttpTransport, OperatorClient, ReqwestTransport, TokenProvider};

use crate::data::{Remote, SessionTokenProvider};

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

    /// Await and apply the next pending result (test helper).
    #[cfg(test)]
    async fn recv_once(&mut self) {
        if let Some(rx) = self.rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            self.apply(result);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use tbo_operator_client::{HttpResponse, HttpTransport, Result, StaticTokenProvider};

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        response: Arc<Mutex<HttpResponse>>,
    }

    impl FakeTransport {
        fn new(status: u16, body: &str) -> Self {
            Self {
                response: Arc::new(Mutex::new(HttpResponse {
                    status,
                    body: body.to_owned(),
                })),
            }
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(self.response.lock().unwrap().clone())
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
}
