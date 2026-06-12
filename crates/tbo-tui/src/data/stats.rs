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
        self.start_load();
    }

    /// Spawn a load for the current window, replacing any in-flight request.
    ///
    /// Replacing `self.rx` discards any result still pending from a previous
    /// request, so the most recently requested window always wins.
    fn start_load(&mut self) {
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
        // A window change always forces a fresh load, even if one is already in
        // flight, so the visible data matches the selected window.
        self.start_load();
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

    use tbo_operator_client::{HttpResponse, HttpTransport, Result, StaticTokenProvider};

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        status: u16,
        ok: bool,
    }

    impl FakeTransport {
        fn ok() -> Self {
            Self {
                status: 200,
                ok: true,
            }
        }

        fn failing(status: u16) -> Self {
            Self { status, ok: false }
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            _path: &str,
            query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            // Echo the requested window back in the body so tests can assert
            // which window's data was loaded.
            let window = query
                .iter()
                .find_map(|(key, value)| (*key == "window").then(|| value.clone()))
                .unwrap_or_else(|| "7d".to_owned());
            let body = if self.ok {
                overview_json(&window)
            } else {
                String::new()
            };
            Ok(HttpResponse {
                status: self.status,
                body,
            })
        }
    }

    fn overview_json(window: &str) -> String {
        format!(
            r#"{{"window":"{window}","rangeEnd":"2026-01-01T00:00:00Z","generatedAt":"2026-01-01T00:00:00Z","timezone":"UTC","calls":{{"total":3,"completed":2,"inProgress":0,"outcomes":{{}},"perDay":[]}},"messages":{{"total":2,"byStatus":{{}}}},"playback":{{"totalPlaybacks":5}},"pickupsHangups":{{"pickups":3,"hangups":3,"digitsDialed":{{}}}},"uploads":{{"succeeded":2,"failed":0}},"topQuestions":[],"hourly":[],"busiest":{{}},"boothBreakdown":[]}}"#
        )
    }

    fn controller(transport: FakeTransport) -> StatsController<FakeTransport, StaticTokenProvider> {
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::new("token"));
        StatsController::new(client)
    }

    #[tokio::test]
    async fn refresh_loads_overview_into_ready() {
        let mut controller = controller(FakeTransport::ok());
        controller.refresh();
        controller.recv_once().await;
        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.calls.total, 3),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cycle_window_advances_and_reloads() {
        let mut controller = controller(FakeTransport::ok());
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
    async fn cycle_window_while_loading_reloads_for_new_window() {
        let mut controller = controller(FakeTransport::ok());
        // Begin a load for the default (Week) window but do not apply it.
        controller.refresh();
        assert!(controller.is_refreshing());

        // Cycling while in flight must start a fresh load for the new window;
        // the result that lands should reflect the new window, not the old.
        controller.cycle_window();
        assert_eq!(controller.window(), StatsWindow::Month);
        controller.recv_once().await;
        match controller.state() {
            Remote::Ready { value, .. } => assert_eq!(value.window, StatsWindow::Month),
            other => panic!("expected Ready for Month window, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_load_becomes_failed_state() {
        let mut controller = controller(FakeTransport::failing(401));
        controller.refresh();
        controller.recv_once().await;
        assert!(matches!(controller.state(), Remote::Failed { .. }));
        assert!(!controller.is_refreshing());
    }
}
