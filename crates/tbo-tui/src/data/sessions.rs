//! Background loading of call sessions and their per-session timelines.
//!
//! The list (`GET /v1/sessions`) loads once on first focus, like the other
//! read screens. In addition, the detail timeline (`GET /v1/sessions/{id}`) is
//! fetched lazily whenever the selected row changes, so the right-hand pane can
//! show the full event timeline without that fetch ever blocking rendering.

use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::{CallSession, CallSessionDetail};
use tbo_operator_client::{HttpTransport, OperatorClient, ReqwestTransport, TokenProvider};

use crate::data::{Remote, SessionTokenProvider};

/// How many sessions to request per load.
const PAGE_LIMIT: u32 = 50;

/// Loads the session list and the selected session's timeline off the UI thread.
pub struct SessionsController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    list: Remote<Vec<CallSession>>,
    selected: usize,
    list_rx: Option<UnboundedReceiver<std::result::Result<Vec<CallSession>, String>>>,
    list_in_flight: bool,
    loaded: bool,
    detail: Remote<CallSessionDetail>,
    detail_for: Option<String>,
    detail_rx: Option<UnboundedReceiver<std::result::Result<CallSessionDetail, String>>>,
    detail_in_flight: bool,
}

impl<T, A> SessionsController<T, A>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Build a controller over the given operator client.
    pub fn new(client: OperatorClient<T, A>) -> Self {
        Self {
            client,
            list: Remote::Idle,
            selected: 0,
            list_rx: None,
            list_in_flight: false,
            loaded: false,
            detail: Remote::Idle,
            detail_for: None,
            detail_rx: None,
            detail_in_flight: false,
        }
    }

    /// The current list load state.
    #[must_use]
    pub fn state(&self) -> &Remote<Vec<CallSession>> {
        &self.list
    }

    /// The current detail (timeline) load state for the selected session.
    #[must_use]
    pub fn detail(&self) -> &Remote<CallSessionDetail> {
        &self.detail
    }

    /// The index of the selected row.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected session, when the list is loaded and non-empty.
    #[must_use]
    pub fn selected_session(&self) -> Option<&CallSession> {
        match &self.list {
            Remote::Ready { value, .. } => value.get(self.selected),
            _ => None,
        }
    }

    /// Whether a list load is currently in flight.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.list_in_flight
    }

    /// Reload the list and force the detail to re-sync for the selection.
    pub fn refresh(&mut self) {
        self.refresh_list();
        self.detail_for = None;
    }

    /// Move the selection to the next row, if any.
    pub fn select_next(&mut self) {
        if let Remote::Ready { value, .. } = &self.list
            && self.selected + 1 < value.len()
        {
            self.selected += 1;
        }
    }

    /// Move the selection to the previous row, if any.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Apply any completed loads, then keep the detail in sync with the
    /// selection and perform the initial list load on first focus.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused && !self.loaded && !self.list_in_flight {
            self.refresh_list();
        }
        if focused {
            self.sync_detail();
        }
    }

    /// Apply any completed loads (non-blocking). Called each tick.
    pub fn drain(&mut self) {
        self.drain_list();
        self.drain_detail();
    }

    /// Trigger a list load unless one is already in flight.
    fn refresh_list(&mut self) {
        if self.list_in_flight {
            return;
        }
        self.list_in_flight = true;
        if matches!(self.list, Remote::Idle | Remote::Failed { .. }) {
            self.list = Remote::Loading;
        }
        let (tx, rx) = unbounded_channel();
        self.list_rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client
                .sessions(None, None, Some(PAGE_LIMIT))
                .await
                .map(|list| list.items)
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Fetch the timeline for the selected session if it is not already loaded
    /// or in flight.
    fn sync_detail(&mut self) {
        let Some(id) = self.selected_session().map(|session| session.id.clone()) else {
            return;
        };
        if self.detail_in_flight || self.detail_for.as_deref() == Some(id.as_str()) {
            return;
        }
        self.detail_for = Some(id.clone());
        self.detail_in_flight = true;
        self.detail = Remote::Loading;
        let (tx, rx) = unbounded_channel();
        self.detail_rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client.session(&id).await.map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Drain the list channel.
    fn drain_list(&mut self) {
        loop {
            let Some(rx) = self.list_rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply_list(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.list_rx = None;
                    self.list_in_flight = false;
                    return;
                }
            }
        }
    }

    /// Drain the detail channel.
    fn drain_detail(&mut self) {
        loop {
            let Some(rx) = self.detail_rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply_detail(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The task ended without sending; surface it as a failure so
                    // the pane is not stuck on "loading" and `r` can retry.
                    self.detail_rx = None;
                    self.detail_in_flight = false;
                    self.detail = Remote::Failed {
                        error: "timeline fetch was cancelled".to_owned(),
                        at: Instant::now(),
                    };
                    return;
                }
            }
        }
    }

    /// Apply a single list result to the visible state.
    fn apply_list(&mut self, result: std::result::Result<Vec<CallSession>, String>) {
        self.list_in_flight = false;
        self.loaded = true;
        self.list_rx = None;
        match result {
            Ok(items) => {
                self.selected = self.selected.min(items.len().saturating_sub(1));
                self.list = Remote::Ready {
                    value: items,
                    fetched_at: Instant::now(),
                };
            }
            Err(error) => {
                self.list = Remote::Failed {
                    error,
                    at: Instant::now(),
                };
            }
        }
    }

    /// Apply a single detail result to the visible state.
    fn apply_detail(&mut self, result: std::result::Result<CallSessionDetail, String>) {
        self.detail_in_flight = false;
        self.detail_rx = None;
        match result {
            Ok(detail) => {
                self.detail = Remote::Ready {
                    value: detail,
                    fetched_at: Instant::now(),
                };
            }
            Err(error) => {
                self.detail = Remote::Failed {
                    error,
                    at: Instant::now(),
                };
            }
        }
    }

    /// Await and apply the next pending list result (test helper).
    #[cfg(test)]
    async fn recv_list_once(&mut self) {
        if let Some(rx) = self.list_rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            self.apply_list(result);
        }
    }

    /// Await and apply the next pending detail result (test helper).
    #[cfg(test)]
    async fn recv_detail_once(&mut self) {
        if let Some(rx) = self.detail_rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            self.apply_detail(result);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use tbo_operator_client::{HttpResponse, HttpTransport, Result, StaticTokenProvider};

    use super::*;

    /// Routes list vs. detail requests by path so a single transport can serve
    /// both `GET /v1/sessions` and `GET /v1/sessions/{id}`.
    #[derive(Clone)]
    struct FakeTransport {
        list: Arc<Mutex<HttpResponse>>,
    }

    impl FakeTransport {
        fn new(list_status: u16, list_body: &str) -> Self {
            Self {
                list: Arc::new(Mutex::new(HttpResponse {
                    status: list_status,
                    body: list_body.to_owned(),
                })),
            }
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            if let Some(id) = path.strip_prefix("/v1/sessions/") {
                return Ok(HttpResponse {
                    status: 200,
                    body: session_detail_json(id),
                });
            }
            Ok(self.list.lock().unwrap().clone())
        }
    }

    fn controller(
        list_status: u16,
        list_body: &str,
    ) -> SessionsController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(list_status, list_body),
            StaticTokenProvider::new("token"),
        );
        SessionsController::new(client)
    }

    fn session_json(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","boothId":"booth-1","bootId":"boot-1","startedAt":"2026-01-01T00:00:00Z"}}"#
        )
    }

    fn session_detail_json(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","boothId":"booth-1","bootId":"boot-1","startedAt":"2026-01-01T00:00:00Z","events":[{{"id":"e1","eventId":"ev1","boothId":"booth-1","bootId":"boot-1","type":"call_started","occurredAt":"2026-01-01T00:00:00Z","receivedAt":"2026-01-01T00:00:01Z"}}]}}"#
        )
    }

    #[tokio::test]
    async fn refresh_loads_sessions_into_ready() {
        let body = format!(
            r#"{{"items":[{},{}],"nextCursor":null}}"#,
            session_json("a"),
            session_json("b")
        );
        let mut controller = controller(200, &body);

        controller.refresh();
        controller.recv_list_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 2),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert_eq!(
            controller.selected_session().map(|s| s.id.as_str()),
            Some("a")
        );
    }

    #[tokio::test]
    async fn detail_tracks_selection() {
        let body = format!(
            r#"{{"items":[{},{}],"nextCursor":null}}"#,
            session_json("a"),
            session_json("b")
        );
        let mut controller = controller(200, &body);
        controller.refresh();
        controller.recv_list_once().await;

        // Sync the detail for the first selection.
        controller.sync_detail();
        controller.recv_detail_once().await;
        match controller.detail() {
            Remote::Ready { value, .. } => {
                assert_eq!(value.session.id, "a");
                assert_eq!(value.events.len(), 1);
            }
            other => panic!("expected Ready, got {other:?}"),
        }

        // Moving the selection causes the next sync to fetch the new timeline.
        controller.select_next();
        controller.sync_detail();
        controller.recv_detail_once().await;
        match controller.detail() {
            Remote::Ready { value, .. } => assert_eq!(value.session.id, "b"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn detail_does_not_refetch_for_same_selection() {
        let body = format!(r#"{{"items":[{}],"nextCursor":null}}"#, session_json("a"));
        let mut controller = controller(200, &body);
        controller.refresh();
        controller.recv_list_once().await;

        controller.sync_detail();
        controller.recv_detail_once().await;
        assert!(matches!(controller.detail(), Remote::Ready { .. }));

        // A second sync for the same selection must be a no-op (no new fetch).
        controller.sync_detail();
        assert!(!controller.detail_in_flight);
    }

    #[tokio::test]
    async fn failed_list_becomes_failed_state() {
        let mut controller = controller(401, "");
        controller.refresh();
        controller.recv_list_once().await;
        assert!(matches!(controller.state(), Remote::Failed { .. }));
        assert!(!controller.is_refreshing());
    }
}
