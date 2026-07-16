//! Background loading of booth status for the Status screen.

use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tracing::warn;

use tbo_core::domain::{BoothStatus, WsEnvelope};
use tbo_operator_client::{HttpTransport, OperatorClient, ReqwestTransport, TokenProvider};

use crate::data::{Remote, SessionTokenProvider};

/// How often the Status screen auto-refreshes while focused.
const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Initial delay before reconnecting the live status socket.
const WS_RECONNECT_INITIAL: Duration = Duration::from_secs(1);
/// Maximum delay between live status socket reconnect attempts.
const WS_RECONNECT_MAX: Duration = Duration::from_secs(30);

/// Tracks booth status off the UI thread and keeps the latest value.
///
/// `refresh` spawns the backup REST poll on the `tokio` runtime; `tick` starts
/// a bearer-authenticated live WebSocket while the screen is focused and also
/// keeps the REST poll running on a fixed cadence to fill gaps.
pub struct StatusController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    state: Remote<BoothStatus>,
    rx: Option<UnboundedReceiver<std::result::Result<BoothStatus, String>>>,
    live_rx: Option<UnboundedReceiver<BoothStatus>>,
    in_flight: bool,
    last_refresh: Option<Instant>,
}

impl<T, A> StatusController<T, A>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Build a controller over the given operator client.
    pub fn new(client: OperatorClient<T, A>) -> Self {
        Self {
            client,
            state: Remote::Idle,
            rx: None,
            live_rx: None,
            in_flight: false,
            last_refresh: None,
        }
    }

    /// The current load state.
    #[must_use]
    pub fn state(&self) -> &Remote<BoothStatus> {
        &self.state
    }

    /// Whether a fetch is currently in flight.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.in_flight
    }

    /// Trigger a status fetch unless one is already in flight.
    pub fn refresh(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        // Only show the bare "loading" state when there is no value to preserve.
        if matches!(self.state, Remote::Idle | Remote::Failed { .. }) {
            self.state = Remote::Loading;
        }
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client.status().await.map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Apply any completed fetch (non-blocking). Called each tick.
    pub fn drain(&mut self) {
        self.drain_poll();
        self.drain_live();
    }

    /// Apply any completed REST poll (non-blocking).
    fn drain_poll(&mut self) {
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The fetch task ended without sending (e.g. it panicked or
                    // was cancelled). Clear the in-flight flag so the controller
                    // can recover and re-poll, rather than wedging forever.
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    /// Advance the controller: apply results, then auto-refresh when the screen
    /// is `focused` and the poll interval has elapsed.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused {
            self.ensure_live();
            if self.is_due(Instant::now()) {
                self.refresh();
            }
        }
    }

    /// Start the live status socket worker if it is not already running.
    fn ensure_live(&mut self) {
        if self.live_rx.is_some() {
            return;
        }
        let (tx, rx) = unbounded_channel();
        self.live_rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut backoff = WS_RECONNECT_INITIAL;
            loop {
                match client.status_stream().await {
                    Ok(mut stream) => {
                        backoff = WS_RECONNECT_INITIAL;
                        while let Some(item) = stream.next().await {
                            match item {
                                Ok(WsEnvelope::Status { status }) => {
                                    if tx.send(status).is_err() {
                                        return;
                                    }
                                }
                                Ok(_) => {}
                                Err(err) => {
                                    warn!(error = %err, "status WebSocket stream ended with error");
                                    break;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        warn!(error = %err, "failed to connect status WebSocket");
                    }
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, WS_RECONNECT_MAX);
            }
        });
    }

    /// Apply any live status frames (non-blocking).
    fn drain_live(&mut self) {
        loop {
            let Some(rx) = self.live_rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(status) => self.apply_status(status, Instant::now()),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.live_rx = None;
                    return;
                }
            }
        }
    }

    /// Whether an auto-refresh is due at `now`.
    fn is_due(&self, now: Instant) -> bool {
        if self.in_flight {
            return false;
        }
        self.last_refresh
            .is_none_or(|last| now.duration_since(last) >= POLL_INTERVAL)
    }

    /// Apply a single fetch result to the visible state.
    fn apply(&mut self, result: std::result::Result<BoothStatus, String>) {
        let now = Instant::now();
        self.in_flight = false;
        self.last_refresh = Some(now);
        self.rx = None;
        match result {
            Ok(value) => self.apply_status(value, now),
            Err(error) => {
                if !matches!(self.state, Remote::Ready { .. }) {
                    self.state = Remote::Failed { error, at: now };
                }
            }
        }
    }

    /// Apply a status update when it is not older than the visible value.
    fn apply_status(&mut self, value: BoothStatus, fetched_at: Instant) {
        if let Remote::Ready { value: current, .. } = &self.state
            && value.updated_at < current.updated_at
        {
            return;
        }
        self.state = Remote::Ready { value, fetched_at };
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

    use tbo_core::domain::BoothState;
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

    fn controller(status: u16, body: &str) -> StatusController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(status, body),
            StaticTokenProvider::anonymous(),
        );
        StatusController::new(client)
    }

    fn booth_status(state: BoothState, updated_at: &str) -> BoothStatus {
        serde_json::from_str(&format!(
            r#"{{"state":"{}","updatedAt":"{updated_at}"}}"#,
            match state {
                BoothState::Idle => "idle",
                BoothState::DialTone => "dialTone",
                BoothState::Dialing => "dialing",
                BoothState::PlayingQuestion => "playingQuestion",
                BoothState::Beep => "beep",
                BoothState::Recording => "recording",
                BoothState::Uploading => "uploading",
                BoothState::PlayingMessage => "playingMessage",
                BoothState::PlayingInstructions => "playingInstructions",
                BoothState::Error => "error",
            }
        ))
        .unwrap()
    }

    #[tokio::test]
    async fn refresh_loads_status_into_ready() {
        let mut controller = controller(
            200,
            r#"{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}"#,
        );

        controller.refresh();
        controller.recv_once().await;

        assert!(matches!(controller.state(), Remote::Ready { .. }));
        assert!(!controller.is_refreshing());
        assert!(controller.last_refresh.is_some());
    }

    #[tokio::test]
    async fn failed_fetch_becomes_failed_state() {
        let mut controller = controller(500, "boom");

        controller.refresh();
        controller.recv_once().await;

        assert!(matches!(controller.state(), Remote::Failed { .. }));
        assert!(!controller.is_refreshing());
    }

    #[test]
    fn live_status_applies_immediately() {
        let mut controller = controller(500, "boom");

        controller.apply_status(
            booth_status(BoothState::Recording, "2026-01-01T00:00:01Z"),
            Instant::now(),
        );

        assert!(matches!(
            controller.state(),
            Remote::Ready { value, .. } if value.state == BoothState::Recording
        ));
    }

    #[test]
    fn stale_poll_does_not_replace_newer_live_status() {
        let mut controller = controller(500, "boom");

        controller.apply_status(
            booth_status(BoothState::Recording, "2026-01-01T00:00:02Z"),
            Instant::now(),
        );
        controller.apply(Ok(booth_status(BoothState::Idle, "2026-01-01T00:00:01Z")));

        assert!(matches!(
            controller.state(),
            Remote::Ready { value, .. } if value.state == BoothState::Recording
        ));
    }

    #[tokio::test]
    async fn refresh_is_ignored_while_in_flight() {
        let mut controller = controller(
            200,
            r#"{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}"#,
        );

        controller.refresh();
        assert!(controller.is_refreshing());
        // A second refresh while one is in flight must not replace the channel.
        controller.refresh();
        controller.recv_once().await;

        assert!(matches!(controller.state(), Remote::Ready { .. }));
    }

    #[test]
    fn is_due_respects_interval_and_in_flight() {
        let mut controller = controller(
            200,
            r#"{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}"#,
        );
        let now = Instant::now();

        // Never refreshed: due immediately.
        assert!(controller.is_due(now));

        // Just refreshed: not due until the interval elapses.
        controller.last_refresh = Some(now);
        assert!(!controller.is_due(now));
        assert!(controller.is_due(now + POLL_INTERVAL));

        // In flight: never due.
        controller.in_flight = true;
        assert!(!controller.is_due(now + POLL_INTERVAL));
    }

    #[tokio::test]
    async fn drain_recovers_when_fetch_task_drops_without_sending() {
        let mut controller = controller(
            200,
            r#"{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}"#,
        );
        // Simulate a fetch task that ended without sending (panic/cancel): an
        // in-flight request whose sender has already been dropped.
        let (tx, rx) = unbounded_channel::<std::result::Result<BoothStatus, String>>();
        drop(tx);
        controller.rx = Some(rx);
        controller.in_flight = true;

        controller.drain();

        assert!(!controller.is_refreshing());
        assert!(controller.rx.is_none());
        // The controller must remain able to re-poll afterwards.
        assert!(controller.is_due(Instant::now()));
    }
}
