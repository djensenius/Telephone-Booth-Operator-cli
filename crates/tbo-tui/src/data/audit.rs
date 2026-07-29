//! Background loading of the audit trail (`GET /v1/audit-logs`).
//!
//! The trail answers "who approved this, from where, and when". It is
//! admin-only on the server, so a non-admin operator sees the load fail with
//! `403` rather than an empty list — the screen is gated the same way Tokens
//! and Debug are, so that should not normally happen.
//!
//! Loading follows the same shape as the other read screens: one load on first
//! focus, `r` to reload, arrows to move the selection. `f` cycles the action
//! family filter, which resets pagination and reloads.

use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::{AuditLogEntry, AuditQuery};
use tbo_operator_client::{HttpTransport, OperatorClient, ReqwestTransport, TokenProvider};

use crate::data::{Remote, SessionTokenProvider};

/// A completed page load: the entries and the cursor for the next one.
type PageResult = std::result::Result<(Vec<AuditLogEntry>, Option<String>), String>;

/// How many entries to request per page.
const PAGE_LIMIT: u32 = 50;

/// The action-family filters offered on the screen, in cycle order. `None` is
/// "everything"; the rest are action-name prefixes the API matches server-side.
const FILTERS: [(Option<&str>, &str); 7] = [
    (None, "all actions"),
    (Some("message."), "messages"),
    (Some("message.approve"), "approvals"),
    (Some("message.reject"), "rejections"),
    (Some("question."), "questions"),
    (Some("apiToken."), "API tokens"),
    (Some("auth.login"), "sign-in"),
];

/// Loads audit entries off the UI thread.
pub struct AuditController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    list: Remote<Vec<AuditLogEntry>>,
    selected: usize,
    filter: usize,
    /// Cursor for the next (older) page, from the last successful load.
    next_cursor: Option<String>,
    /// Cursor sent with the in-flight request, if it is a "load more".
    loading_more: bool,
    rx: Option<UnboundedReceiver<PageResult>>,
    in_flight: bool,
    loaded: bool,
}

impl<T, A> AuditController<T, A>
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
            filter: 0,
            next_cursor: None,
            loading_more: false,
            rx: None,
            in_flight: false,
            loaded: false,
        }
    }

    /// The current load state.
    #[must_use]
    pub fn state(&self) -> &Remote<Vec<AuditLogEntry>> {
        &self.list
    }

    /// The index of the selected row.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected entry, when the list is loaded and non-empty.
    #[must_use]
    pub fn selected_entry(&self) -> Option<&AuditLogEntry> {
        match &self.list {
            Remote::Ready { value, .. } => value.get(self.selected),
            _ => None,
        }
    }

    /// The label of the active action filter.
    #[must_use]
    pub fn filter_label(&self) -> &'static str {
        FILTERS[self.filter].1
    }

    /// Whether older entries remain beyond what has been loaded.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.next_cursor.is_some()
    }

    /// Whether a load is currently in flight.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.in_flight
    }

    /// Reload from the first page, keeping the active filter.
    pub fn refresh(&mut self) {
        self.next_cursor = None;
        self.load(false);
    }

    /// Move to the next action filter and reload from the first page.
    pub fn cycle_filter(&mut self) {
        self.filter = (self.filter + 1) % FILTERS.len();
        self.selected = 0;
        self.refresh();
    }

    /// Append the next (older) page, if there is one.
    pub fn load_more(&mut self) {
        if self.next_cursor.is_some() {
            self.load(true);
        }
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

    /// Apply any completed load, then perform the initial load on first focus.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused && !self.loaded && !self.in_flight {
            self.load(false);
        }
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
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    /// Trigger a load unless one is already in flight. `more` appends the next
    /// page instead of replacing the list.
    fn load(&mut self, more: bool) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        self.loading_more = more;
        if !more && matches!(self.list, Remote::Idle | Remote::Failed { .. }) {
            self.list = Remote::Loading;
        }
        let filter = AuditQuery {
            action: FILTERS[self.filter].0.map(str::to_owned),
            actor_type: None,
            actor_user_id: None,
            cursor: if more { self.next_cursor.clone() } else { None },
            limit: Some(PAGE_LIMIT),
        };
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client
                .audit_logs(&filter)
                .await
                .map(|page| (page.items, page.next_cursor))
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Apply a single load result to the visible state.
    fn apply(&mut self, result: std::result::Result<(Vec<AuditLogEntry>, Option<String>), String>) {
        let more = self.loading_more;
        self.in_flight = false;
        self.loading_more = false;
        self.loaded = true;
        self.rx = None;
        match result {
            Ok((items, next_cursor)) => {
                self.next_cursor = next_cursor;
                let value = match (&mut self.list, more) {
                    (Remote::Ready { value, .. }, true) => {
                        let mut existing = std::mem::take(value);
                        existing.extend(items);
                        existing
                    }
                    _ => items,
                };
                self.selected = self.selected.min(value.len().saturating_sub(1));
                self.list = Remote::Ready {
                    value,
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

    /// Records the query it was called with so filter and cursor propagation
    /// can be asserted, and replays a canned page.
    #[derive(Clone)]
    struct FakeTransport {
        response: Arc<Mutex<HttpResponse>>,
        last_query: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl FakeTransport {
        fn new(status: u16, body: &str) -> Self {
            Self {
                response: Arc::new(Mutex::new(HttpResponse {
                    status,
                    body: body.to_owned(),
                })),
                last_query: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn set_body(&self, body: &str) {
            self.response.lock().unwrap().body = body.to_owned();
        }

        fn query_value(&self, key: &str) -> Option<String> {
            self.last_query
                .lock()
                .unwrap()
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            _path: &str,
            query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            *self.last_query.lock().unwrap() = query
                .iter()
                .map(|(name, value)| ((*name).to_owned(), value.clone()))
                .collect();
            Ok(self.response.lock().unwrap().clone())
        }
    }

    fn entry_json(id: &str, action: &str) -> String {
        format!(
            r#"{{"id":"{id}","action":"{action}","actorType":"operator","actorLabel":"op@example.com","ip":"203.0.113.7","method":"POST","path":"/v1/x","statusCode":200,"createdAt":"2026-01-01T00:00:00Z"}}"#
        )
    }

    fn page_json(entries: &[(&str, &str)], next_cursor: Option<&str>) -> String {
        let items = entries
            .iter()
            .map(|(id, action)| entry_json(id, action))
            .collect::<Vec<_>>()
            .join(",");
        let cursor = next_cursor.map_or_else(|| "null".to_owned(), |c| format!("\"{c}\""));
        format!(r#"{{"items":[{items}],"nextCursor":{cursor}}}"#)
    }

    fn controller(transport: FakeTransport) -> AuditController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::new("token"));
        AuditController::new(client)
    }

    #[tokio::test]
    async fn refresh_loads_entries_into_ready() {
        let transport = FakeTransport::new(200, &page_json(&[("a", "message.approve")], None));
        let mut controller = controller(transport);

        controller.refresh();
        controller.recv_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 1),
            other => panic!("expected Ready, got {other:?}"),
        }
        let entry = controller.selected_entry().unwrap();
        assert_eq!(entry.actor_label, "op@example.com");
        assert_eq!(entry.ip.as_deref(), Some("203.0.113.7"));
        assert!(!controller.has_more());
    }

    #[tokio::test]
    async fn cycling_the_filter_sends_an_action_prefix() {
        let transport = FakeTransport::new(200, &page_json(&[], None));
        let mut controller = controller(transport.clone());
        controller.refresh();
        controller.recv_once().await;
        assert_eq!(transport.query_value("action"), None);

        controller.cycle_filter();
        controller.recv_once().await;
        assert_eq!(transport.query_value("action").as_deref(), Some("message."));
        assert_eq!(controller.filter_label(), "messages");
    }

    #[tokio::test]
    async fn load_more_appends_the_next_page_with_the_cursor() {
        let transport = FakeTransport::new(
            200,
            &page_json(&[("a", "message.approve")], Some("cursor-1")),
        );
        let mut controller = controller(transport.clone());
        controller.refresh();
        controller.recv_once().await;
        assert!(controller.has_more());

        transport.set_body(&page_json(&[("b", "message.reject")], None));
        controller.load_more();
        controller.recv_once().await;

        assert_eq!(transport.query_value("cursor").as_deref(), Some("cursor-1"));
        match controller.state() {
            Remote::Ready { value, .. } => {
                assert_eq!(value.len(), 2);
                assert_eq!(value[1].action, "message.reject");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!controller.has_more());
    }

    #[tokio::test]
    async fn load_more_is_a_no_op_without_a_cursor() {
        let transport = FakeTransport::new(200, &page_json(&[("a", "auth.login")], None));
        let mut controller = controller(transport);
        controller.refresh();
        controller.recv_once().await;

        controller.load_more();
        assert!(!controller.is_refreshing());
    }

    #[tokio::test]
    async fn a_forbidden_load_becomes_failed_state() {
        let mut controller = controller(FakeTransport::new(403, ""));
        controller.refresh();
        controller.recv_once().await;
        assert!(matches!(controller.state(), Remote::Failed { .. }));
        assert!(!controller.is_refreshing());
    }
}
