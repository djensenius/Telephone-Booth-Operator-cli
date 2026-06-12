//! Background polling of `GET /v1/status` for the Status screen.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::BoothStatus;
use tbo_operator_client::{HttpTransport, OperatorClient, ReqwestTransport, TokenProvider};

use crate::data::{Remote, SessionTokenProvider};

/// How often the Status screen auto-refreshes while focused.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Polls booth status off the UI thread and tracks the latest value.
///
/// `refresh` spawns a fetch on the `tokio` runtime; `drain` (called each tick)
/// applies the completed result, and `tick` additionally re-polls on a fixed
/// cadence while the screen is focused.
pub struct StatusController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    state: Remote<BoothStatus>,
    rx: Option<UnboundedReceiver<std::result::Result<BoothStatus, String>>>,
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
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    return;
                }
            }
        }
    }

    /// Advance the controller: apply results, then auto-refresh when the screen
    /// is `focused` and the poll interval has elapsed.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused && self.is_due(Instant::now()) {
            self.refresh();
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
        self.state = match result {
            Ok(value) => Remote::Ready {
                value,
                fetched_at: now,
            },
            Err(error) => Remote::Failed { error, at: now },
        };
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

    fn controller(status: u16, body: &str) -> StatusController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(status, body),
            StaticTokenProvider::anonymous(),
        );
        StatusController::new(client)
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
}
