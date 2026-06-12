//! Background loading of `GET /v1/events` for the Events screen.

use std::time::Instant;

use futures::StreamExt;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::JoinHandle;

use tbo_core::domain::BoothEventRecord;
use tbo_operator_client::{
    EventQuery, HttpTransport, OperatorClient, ReqwestTransport, SseTransport, TokenProvider,
};

use crate::data::{Remote, SessionTokenProvider};

/// How many events to request per load.
const PAGE_LIMIT: u32 = 50;

/// Upper bound on retained events while live-tailing, so a long-running follow
/// session cannot grow the buffer without bound.
const MAX_EVENTS: usize = 500;

/// Loads the event log off the UI thread and tracks the selected row.
///
/// Like the other list screens it loads once when first focused and thereafter
/// only on demand, so a reload never disrupts the current selection.
pub struct EventsController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    state: Remote<Vec<BoothEventRecord>>,
    selected: usize,
    rx: Option<UnboundedReceiver<std::result::Result<Vec<BoothEventRecord>, String>>>,
    in_flight: bool,
    loaded: bool,
    following: bool,
    stream_rx: Option<UnboundedReceiver<std::result::Result<BoothEventRecord, String>>>,
    stream_task: Option<JoinHandle<()>>,
}

impl<T, A> EventsController<T, A>
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
            following: false,
            stream_rx: None,
            stream_task: None,
        }
    }

    /// The current load state.
    #[must_use]
    pub fn state(&self) -> &Remote<Vec<BoothEventRecord>> {
        &self.state
    }

    /// The index of the selected row.
    #[must_use]
    pub fn selected_index(&self) -> usize {
        self.selected
    }

    /// The selected event, when the list is loaded and non-empty.
    #[must_use]
    pub fn selected_event(&self) -> Option<&BoothEventRecord> {
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

    /// Whether the live tail (follow mode) is currently active.
    #[must_use]
    pub fn is_following(&self) -> bool {
        self.following
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
        let query = EventQuery {
            limit: Some(PAGE_LIMIT),
            ..EventQuery::default()
        };
        tokio::spawn(async move {
            let result = client
                .events(&query)
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
        self.drain_stream();
        if focused && !self.loaded && !self.in_flight {
            self.refresh();
        }
    }

    /// Drain any live-tail events delivered since the last tick.
    fn drain_stream(&mut self) {
        loop {
            let Some(rx) = self.stream_rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(Ok(record)) => self.merge_live(record),
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    // The stream errored or closed; leave follow mode.
                    self.stop_follow();
                    return;
                }
                Err(TryRecvError::Empty) => return,
            }
        }
    }

    /// Merge a single live event into the list, newest first and deduplicated.
    fn merge_live(&mut self, record: BoothEventRecord) {
        if let Remote::Ready { value, fetched_at } = &mut self.state {
            if value.iter().any(|existing| existing.id == record.id) {
                return;
            }
            value.insert(0, record);
            value.truncate(MAX_EVENTS);
            *fetched_at = Instant::now();
            let len = value.len();
            // Keep the highlighted row on the same event as new ones arrive,
            // but let a top selection ride the newest event.
            if self.selected > 0 {
                self.selected = (self.selected + 1).min(len.saturating_sub(1));
            }
        } else {
            self.state = Remote::Ready {
                value: vec![record],
                fetched_at: Instant::now(),
            };
            self.selected = 0;
            self.loaded = true;
        }
    }

    /// Stop live-tailing and release the background streaming task.
    fn stop_follow(&mut self) {
        self.following = false;
        if let Some(task) = self.stream_task.take() {
            task.abort();
        }
        self.stream_rx = None;
    }

    /// Apply a single load result to the visible state.
    fn apply(&mut self, result: std::result::Result<Vec<BoothEventRecord>, String>) {
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

    /// Await and merge the next live-tail event (test helper).
    #[cfg(test)]
    async fn recv_stream_once(&mut self) {
        if let Some(rx) = self.stream_rx.as_mut()
            && let Some(result) = rx.recv().await
        {
            match result {
                Ok(record) => self.merge_live(record),
                Err(_) => self.stop_follow(),
            }
        }
    }
}

impl<T, A> EventsController<T, A>
where
    T: SseTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Toggle the live event tail (`GET /v1/events/stream`) on or off.
    ///
    /// Starting spawns a background task that forwards each `booth-event` frame
    /// into the list (newest first); stopping aborts that task. New events are
    /// merged on [`tick`](Self::tick).
    pub fn toggle_follow(&mut self) {
        if self.following {
            self.stop_follow();
            return;
        }
        self.following = true;
        let (tx, rx) = unbounded_channel();
        self.stream_rx = Some(rx);
        let client = self.client.clone();
        let task = tokio::spawn(async move {
            match client.events_stream(&EventQuery::default()).await {
                Ok(mut stream) => {
                    while let Some(item) = stream.next().await {
                        if tx.send(item.map_err(|err| err.to_string())).is_err() {
                            break;
                        }
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err.to_string()));
                }
            }
        });
        self.stream_task = Some(task);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use tbo_operator_client::{
        ByteStream, HttpResponse, HttpTransport, Result, SseTransport, StaticTokenProvider,
    };

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        response: Arc<Mutex<HttpResponse>>,
        sse: Arc<Mutex<Vec<u8>>>,
    }

    impl FakeTransport {
        fn new(status: u16, body: &str) -> Self {
            Self {
                response: Arc::new(Mutex::new(HttpResponse {
                    status,
                    body: body.to_owned(),
                })),
                sse: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_sse(self, frames: &str) -> Self {
            *self.sse.lock().unwrap() = frames.as_bytes().to_vec();
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
            Ok(self.response.lock().unwrap().clone())
        }
    }

    impl SseTransport for FakeTransport {
        async fn get_sse(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<ByteStream> {
            let bytes = self.sse.lock().unwrap().clone();
            Ok(futures::stream::iter(vec![Ok(bytes)]).boxed())
        }
    }

    fn controller(status: u16, body: &str) -> EventsController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(status, body),
            StaticTokenProvider::new("token"),
        );
        EventsController::new(client)
    }

    fn event_json(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","eventId":"{id}","boothId":"booth-1","bootId":"boot-1","type":"call_started","occurredAt":"2026-01-01T00:00:00Z","receivedAt":"2026-01-01T00:00:01Z"}}"#
        )
    }

    #[tokio::test]
    async fn refresh_loads_events_into_ready() {
        let body = format!(r#"{{"items":[{},{}]}}"#, event_json("a"), event_json("b"));
        let mut controller = controller(200, &body);

        controller.refresh();
        controller.recv_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 2),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert_eq!(
            controller.selected_event().map(|e| e.id.as_str()),
            Some("a")
        );
    }

    #[tokio::test]
    async fn selection_moves_and_clamps() {
        let body = format!(r#"{{"items":[{},{}]}}"#, event_json("a"), event_json("b"));
        let mut controller = controller(200, &body);
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
    async fn reload_clamps_selection_to_shorter_list() {
        let body = format!(r#"{{"items":[{},{}]}}"#, event_json("a"), event_json("b"));
        let mut controller = controller(200, &body);
        controller.refresh();
        controller.recv_once().await;
        controller.select_next();
        assert_eq!(controller.selected_index(), 1);

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
    async fn follow_merges_live_events_to_the_front() {
        let body = format!(r#"{{"items":[{}]}}"#, event_json("a"));
        let frames = format!(
            "event: ready\ndata: ok\n\nid: live-1\nevent: booth-event\ndata: {}\n\n",
            event_json("live-1")
        );
        let client = OperatorClient::with_transport(
            FakeTransport::new(200, &body).with_sse(&frames),
            StaticTokenProvider::new("token"),
        );
        let mut controller = EventsController::new(client);

        controller.refresh();
        controller.recv_once().await;
        assert_eq!(
            controller.selected_event().map(|e| e.id.as_str()),
            Some("a")
        );

        controller.toggle_follow();
        assert!(controller.is_following());
        controller.recv_stream_once().await;

        match controller.state() {
            Remote::Ready { value, .. } => {
                assert_eq!(value.len(), 2);
                assert_eq!(value[0].id, "live-1");
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn merge_live_deduplicates_by_id() {
        let body = format!(r#"{{"items":[{}]}}"#, event_json("dup"));
        let mut controller = controller(200, &body);
        controller.refresh();
        controller.recv_once().await;

        controller.merge_live(serde_json::from_str(&event_json("dup")).unwrap());
        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.len(), 1),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn toggling_follow_twice_stops() {
        let mut controller = controller(200, r#"{"items":[]}"#);
        controller.toggle_follow();
        assert!(controller.is_following());
        controller.toggle_follow();
        assert!(!controller.is_following());
    }
}
