//! Background loading of `GET /v1/stats/overview` for the Statistics screen.

use std::time::Instant;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::{StatsOverview, StatsWindow};
use tbo_operator_client::{HttpTransport, OperatorClient, ReqwestTransport, TokenProvider};

use crate::data::{Remote, SessionTokenProvider};

/// Aggregation windows in cycle order (matches the tab order shown in the UI).
const WINDOW_CYCLE: [StatsWindow; 4] = [
    StatsWindow::Day,
    StatsWindow::Week,
    StatsWindow::Month,
    StatsWindow::All,
];

/// Loads the statistics overview off the UI thread for the selected window.
///
/// Like the other read screens it loads once when first focused and thereafter
/// only on demand; changing the window reloads immediately.
pub struct StatsController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    window: StatsWindow,
    state: Remote<StatsOverview>,
    rx: Option<UnboundedReceiver<std::result::Result<StatsOverview, String>>>,
    in_flight: bool,
    loaded: bool,
}

impl<T, A> StatsController<T, A>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Build a controller over the given operator client, defaulting to the
    /// 7-day window the server uses by default.
    pub fn new(client: OperatorClient<T, A>) -> Self {
        Self {
            client,
            window: StatsWindow::Week,
            state: Remote::Idle,
            rx: None,
            in_flight: false,
            loaded: false,
        }
    }

    /// The current load state.
    #[must_use]
    pub fn state(&self) -> &Remote<StatsOverview> {
        &self.state
    }

    /// The currently selected aggregation window.
    #[must_use]
    pub fn window(&self) -> StatsWindow {
        self.window
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
        let window = self.window;
        tokio::spawn(async move {
            let result = client
                .stats_overview(Some(window))
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Advance to the next aggregation window and reload.
    pub fn cycle_window(&mut self) {
        let index = WINDOW_CYCLE
            .iter()
            .position(|window| *window == self.window)
            .unwrap_or(0);
        self.window = WINDOW_CYCLE[(index + 1) % WINDOW_CYCLE.len()];
        // A window change forces a reload even when one is not yet in flight.
        if !self.in_flight {
            self.refresh();
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

    /// Advance the controller: apply results, then perform the initial load the
    /// first time the screen is focused.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused && !self.loaded && !self.in_flight {
            self.refresh();
        }
    }

    /// Apply a single load result to the visible state.
    fn apply(&mut self, result: std::result::Result<StatsOverview, String>) {
        self.in_flight = false;
        self.loaded = true;
        self.rx = None;
        match result {
            Ok(overview) => {
                self.state = Remote::Ready {
                    value: overview,
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

    fn overview_json() -> &'static str {
        r#"{"window":"7d","rangeEnd":"2026-01-01T00:00:00Z","generatedAt":"2026-01-01T00:00:00Z","timezone":"UTC","calls":{"total":3,"completed":2,"inProgress":0,"outcomes":{},"perDay":[]},"messages":{"total":2,"byStatus":{}},"playback":{"totalPlaybacks":5},"pickupsHangups":{"pickups":3,"hangups":3,"digitsDialed":{}},"uploads":{"succeeded":2,"failed":0},"topQuestions":[],"hourly":[],"busiest":{},"boothBreakdown":[]}"#
    }

    fn controller(status: u16, body: &str) -> StatsController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(
            FakeTransport::new(status, body),
            StaticTokenProvider::new("token"),
        );
        StatsController::new(client)
    }

    #[tokio::test]
    async fn refresh_loads_overview_into_ready() {
        let mut controller = controller(200, overview_json());
        controller.refresh();
        controller.recv_once().await;
        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.calls.total, 3),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cycle_window_advances_and_reloads() {
        let mut controller = controller(200, overview_json());
        assert_eq!(controller.window(), StatsWindow::Week);

        controller.cycle_window();
        assert_eq!(controller.window(), StatsWindow::Month);
        assert!(controller.is_refreshing());
        controller.recv_once().await;
        assert!(!controller.is_refreshing());

        controller.cycle_window();
        assert_eq!(controller.window(), StatsWindow::All);
        controller.recv_once().await;

        controller.cycle_window();
        assert_eq!(controller.window(), StatsWindow::Day);
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
