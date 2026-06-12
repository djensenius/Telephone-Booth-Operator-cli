//! Background scraping of the booth `/metrics` endpoint for the btm-style
//! System Health dashboard.
//!
//! Unlike the operator data controllers (which cache a single latest value),
//! this controller rolls every successful scrape into a [`MetricsHistory`] so
//! the dashboard can plot rolling time-series charts. It otherwise follows the
//! same shape: `refresh` spawns a scrape off the UI thread, `drain` applies
//! completed scrapes each tick, and `tick` re-scrapes on a fixed cadence while
//! the screen is focused.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_booth_client::{BoothClient, BoothTransport, ReqwestBoothTransport, Result as BoothResult};
use tbo_core::config::BoothConfig;
use tbo_metrics::{BoothMetrics, MetricsHistory};

/// How often the dashboard scrapes the booth `/metrics` endpoint while focused.
/// A two-second cadence keeps the charts lively without hammering the booth
/// over its Tailscale link.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Samples retained per chart series (~8 minutes at the scrape cadence).
const HISTORY_CAPACITY: usize = 240;

/// The result of one completed scrape: the instant the data was read and the
/// parsed snapshot, or a human-readable error.
type ScrapeResult = std::result::Result<(Instant, BoothMetrics), String>;

/// Scrapes a single booth's Prometheus `/metrics` off the UI thread and rolls
/// the results into a [`MetricsHistory`] for the dashboard to chart.
pub struct SystemHealthController<T = ReqwestBoothTransport>
where
    T: BoothTransport + Clone + Send + Sync + 'static,
{
    label: String,
    client: BoothClient<T>,
    history: MetricsHistory,
    rx: Option<UnboundedReceiver<ScrapeResult>>,
    in_flight: bool,
    last_refresh: Option<Instant>,
    last_ok: Option<Instant>,
    last_error: Option<(String, Instant)>,
    samples: usize,
}

impl SystemHealthController<ReqwestBoothTransport> {
    /// Build a controller for `booth` using the default reqwest transport.
    ///
    /// # Errors
    /// Returns an error when the booth HTTP client cannot be constructed.
    pub fn from_config(booth: &BoothConfig) -> BoothResult<Self> {
        let client = BoothClient::connect(
            booth.debug_base_url.clone(),
            booth.debug_token.clone(),
            booth.pinned_sha256.as_deref(),
        )?;
        let label = booth.name.clone().unwrap_or_else(|| booth.id.clone());
        Ok(Self::new(label, client))
    }
}

impl<T> SystemHealthController<T>
where
    T: BoothTransport + Clone + Send + Sync + 'static,
{
    /// Build a controller over the given booth client, labelled for display.
    pub fn new(label: String, client: BoothClient<T>) -> Self {
        Self {
            label,
            client,
            history: MetricsHistory::new(HISTORY_CAPACITY),
            rx: None,
            in_flight: false,
            last_refresh: None,
            last_ok: None,
            last_error: None,
            samples: 0,
        }
    }

    /// The display label for the booth being charted.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The rolling metrics history backing the charts.
    #[must_use]
    pub fn history(&self) -> &MetricsHistory {
        &self.history
    }

    /// Whether a scrape is currently in flight.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.in_flight
    }

    /// The number of successful scrapes recorded so far.
    #[must_use]
    pub fn samples(&self) -> usize {
        self.samples
    }

    /// When the most recent successful scrape completed.
    #[must_use]
    pub fn last_ok(&self) -> Option<Instant> {
        self.last_ok
    }

    /// The most recent scrape error and when it occurred, if the last attempt
    /// failed.
    #[must_use]
    pub fn last_error(&self) -> Option<&(String, Instant)> {
        self.last_error.as_ref()
    }

    /// Trigger a scrape unless one is already in flight.
    pub fn refresh(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let result = client
                .metrics()
                .await
                .map(|metrics| (Instant::now(), metrics))
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
    }

    /// Apply any completed scrape (non-blocking). Called each tick.
    pub fn drain(&mut self) {
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(result) => self.apply(result),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The scrape task ended without sending (panic/cancel).
                    // Clear the in-flight flag so the controller can recover and
                    // re-scrape rather than wedging forever.
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    /// Advance the controller: apply results, then auto-scrape when the screen
    /// is `focused` and the poll interval has elapsed.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        if focused && self.is_due(Instant::now()) {
            self.refresh();
        }
    }

    /// Whether an auto-scrape is due at `now`.
    fn is_due(&self, now: Instant) -> bool {
        if self.in_flight {
            return false;
        }
        self.last_refresh
            .is_none_or(|last| now.duration_since(last) >= POLL_INTERVAL)
    }

    /// Apply a single scrape result, rolling success into the history.
    fn apply(&mut self, result: ScrapeResult) {
        self.in_flight = false;
        self.last_refresh = Some(Instant::now());
        self.rx = None;
        match result {
            Ok((at, metrics)) => {
                self.history.record(at, metrics);
                self.samples += 1;
                self.last_ok = Some(at);
                self.last_error = None;
            }
            Err(error) => {
                self.last_error = Some((error, Instant::now()));
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
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use std::future::Future;
    use std::sync::{Arc, Mutex};

    use tbo_booth_client::{BoothTransport, HttpResponse, Result};

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

        /// Replace the canned response returned by subsequent requests.
        fn set(&self, status: u16, body: &str) {
            *self.response.lock().unwrap() = HttpResponse {
                status,
                body: body.to_owned(),
            };
        }
    }

    impl BoothTransport for FakeTransport {
        fn get(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> impl Future<Output = Result<HttpResponse>> + Send {
            let response = self.response.lock().unwrap().clone();
            async move { Ok(response) }
        }

        fn post(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
            _json_body: Option<&str>,
        ) -> impl Future<Output = Result<HttpResponse>> + Send {
            let response = self.response.lock().unwrap().clone();
            async move { Ok(response) }
        }
    }

    const METRICS_BODY: &str =
        "booth_cpu_usage_ratio 0.5\nbooth_memory_used_bytes 250\nbooth_memory_total_bytes 1000\n";

    fn controller(status: u16, body: &str) -> SystemHealthController<FakeTransport> {
        let client = BoothClient::with_transport(FakeTransport::new(status, body), None);
        SystemHealthController::new("booth-1".to_owned(), client)
    }

    /// Build a controller plus a handle to its transport so a test can swap the
    /// canned response between scrapes.
    fn controller_with_handle(
        status: u16,
        body: &str,
    ) -> (SystemHealthController<FakeTransport>, FakeTransport) {
        let transport = FakeTransport::new(status, body);
        let client = BoothClient::with_transport(transport.clone(), None);
        (
            SystemHealthController::new("booth-1".to_owned(), client),
            transport,
        )
    }

    #[tokio::test]
    async fn refresh_records_into_history() {
        let mut controller = controller(200, METRICS_BODY);

        controller.refresh();
        controller.recv_once().await;

        assert_eq!(controller.samples(), 1);
        assert!(!controller.is_refreshing());
        assert!(controller.last_ok().is_some());
        assert!(controller.last_error().is_none());
        assert_eq!(controller.history().cpu_usage().to_vec(), vec![0.5]);
        assert_eq!(controller.history().memory_ratio().to_vec(), vec![0.25]);
    }

    #[tokio::test]
    async fn failed_scrape_sets_error_without_recording() {
        let mut controller = controller(500, "boom");

        controller.refresh();
        controller.recv_once().await;

        assert_eq!(controller.samples(), 0);
        assert!(!controller.is_refreshing());
        assert!(controller.last_error().is_some());
        assert!(controller.history().cpu_usage().is_empty());
    }

    #[tokio::test]
    async fn successful_scrape_clears_prior_error() {
        let (mut controller, transport) = controller_with_handle(500, "boom");
        controller.refresh();
        controller.recv_once().await;
        assert!(controller.last_error().is_some());

        transport.set(200, METRICS_BODY);
        controller.refresh();
        controller.recv_once().await;

        assert!(controller.last_error().is_none());
        assert_eq!(controller.samples(), 1);
    }

    #[test]
    fn is_due_respects_interval_and_in_flight() {
        let mut controller = controller(200, METRICS_BODY);
        let now = Instant::now();

        assert!(controller.is_due(now));

        controller.last_refresh = Some(now);
        assert!(!controller.is_due(now));
        assert!(controller.is_due(now + POLL_INTERVAL));

        controller.in_flight = true;
        assert!(!controller.is_due(now + POLL_INTERVAL));
    }

    #[tokio::test]
    async fn drain_recovers_when_scrape_task_drops_without_sending() {
        let mut controller = controller(200, METRICS_BODY);
        let (tx, rx) = unbounded_channel::<ScrapeResult>();
        drop(tx);
        controller.rx = Some(rx);
        controller.in_flight = true;

        controller.drain();

        assert!(!controller.is_refreshing());
        assert!(controller.rx.is_none());
        assert!(controller.is_due(Instant::now()));
    }
}
