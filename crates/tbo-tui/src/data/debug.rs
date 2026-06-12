//! Background polling — and optional live telemetry streaming — of the booth
//! debug server for the Debug panel.
//!
//! Mirrors the [`SystemHealthController`](super::SystemHealthController) shape:
//! `refresh` spawns the network work off the UI thread, `drain` applies a
//! completed round each tick, and `tick` re-polls on a fixed cadence while the
//! screen is focused. One round fetches the state, GPIO, audio, logs, and
//! config endpoints concurrently; each endpoint's result is independent so a
//! single failure leaves the other panels intact (the last good value is kept).
//!
//! The panel can additionally subscribe to the booth's telemetry WebSocket
//! (`toggle_live`): a background task forwards each decoded record over a
//! channel and `drain` folds it into the cached snapshots, so the panels track
//! booth activity in real time. While live the REST poll is suspended (after an
//! initial seed) and telemetry drives the state, GPIO, audio, and log panels,
//! mirroring the web client's preference for the socket over polling.

use std::time::{Duration, Instant, SystemTime};

use futures::StreamExt;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tokio::task::JoinHandle;

use tbo_booth_client::{
    AudioLevel, AudioMeterSnapshot, BoothClient, BoothTransport, ConfigRedacted, GpioEdge,
    GpioPinSnapshot, GpioSnapshot, LogEntry, ReqwestBoothTransport, Result as BoothResult,
    StatusSnapshot, TelemetryEvent, TelemetryRecord, connect_telemetry,
};
use tbo_core::config::BoothConfig;

/// How often the panel re-polls the booth while focused.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Maximum number of log lines requested per poll.
const LOG_LIMIT: usize = 200;

/// Tracing levels cycled through by [`DebugController::cycle_log_level`], from
/// most to least severe.
const LOG_LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

/// The outcome of one polling round: each endpoint's result captured
/// independently so a single failure doesn't blank the rest of the panel.
struct DebugFetch {
    /// `GET /v1/state`.
    state: Result<StatusSnapshot, String>,
    /// `GET /v1/gpio`.
    gpio: Result<GpioSnapshot, String>,
    /// `GET /v1/audio`.
    audio: Result<AudioMeterSnapshot, String>,
    /// `GET /v1/logs`.
    logs: Result<Vec<LogEntry>, String>,
    /// `GET /v1/config`.
    config: Result<ConfigRedacted, String>,
}

/// Polls a single booth's debug REST snapshots for the Debug panel.
pub struct DebugController<T = ReqwestBoothTransport>
where
    T: BoothTransport + Clone + Send + Sync + 'static,
{
    label: String,
    base_url: String,
    pinned_sha256: Option<String>,
    token: Option<String>,
    client: BoothClient<T>,
    log_level: String,
    rx: Option<UnboundedReceiver<DebugFetch>>,
    in_flight: bool,
    last_refresh: Option<Instant>,
    last_ok: Option<Instant>,
    last_error: Option<(String, Instant)>,
    state: Option<StatusSnapshot>,
    gpio: Option<GpioSnapshot>,
    audio: Option<AudioMeterSnapshot>,
    logs: Vec<LogEntry>,
    config: Option<ConfigRedacted>,
    live: bool,
    stream_rx: Option<UnboundedReceiver<std::result::Result<TelemetryRecord, String>>>,
    stream_task: Option<JoinHandle<()>>,
    last_record_id: Option<u64>,
    live_error: Option<(String, Instant)>,
}

impl DebugController<ReqwestBoothTransport> {
    /// Build a controller for `booth` using the default reqwest transport,
    /// pinning LAN TLS when the booth is configured with a fingerprint.
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
        Ok(Self::new(
            label,
            booth.debug_base_url.clone(),
            booth.debug_token.clone(),
            booth.pinned_sha256.clone(),
            client,
        ))
    }
}

impl<T> DebugController<T>
where
    T: BoothTransport + Clone + Send + Sync + 'static,
{
    /// Build a controller over the given booth client, labelled for display.
    pub fn new(
        label: String,
        base_url: String,
        token: Option<String>,
        pinned_sha256: Option<String>,
        client: BoothClient<T>,
    ) -> Self {
        Self {
            label,
            base_url,
            pinned_sha256,
            token,
            client,
            log_level: "info".to_owned(),
            rx: None,
            in_flight: false,
            last_refresh: None,
            last_ok: None,
            last_error: None,
            state: None,
            gpio: None,
            audio: None,
            logs: Vec::new(),
            config: None,
            live: false,
            stream_rx: None,
            stream_task: None,
            last_record_id: None,
            live_error: None,
        }
    }

    /// The display label for the booth.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// The booth debug base URL being polled.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// The configured pinned certificate fingerprint, if any.
    #[must_use]
    pub fn pinned_sha256(&self) -> Option<&str> {
        self.pinned_sha256.as_deref()
    }

    /// The current log-level filter.
    #[must_use]
    pub fn log_level(&self) -> &str {
        &self.log_level
    }

    /// Whether a poll is currently in flight.
    #[must_use]
    pub fn is_refreshing(&self) -> bool {
        self.in_flight
    }

    /// Whether the live telemetry subscription is currently active.
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.live
    }

    /// The most recent live-stream error and when it occurred, if the socket
    /// failed or closed.
    #[must_use]
    pub fn live_error(&self) -> Option<&(String, Instant)> {
        self.live_error.as_ref()
    }

    /// When the most recent fully-successful poll completed.
    #[must_use]
    pub fn last_ok(&self) -> Option<Instant> {
        self.last_ok
    }

    /// The most recent poll error(s) and when they occurred, if the last round
    /// had any failing endpoint.
    #[must_use]
    pub fn last_error(&self) -> Option<&(String, Instant)> {
        self.last_error.as_ref()
    }

    /// The latest state snapshot.
    #[must_use]
    pub fn state(&self) -> Option<&StatusSnapshot> {
        self.state.as_ref()
    }

    /// The latest GPIO snapshot.
    #[must_use]
    pub fn gpio(&self) -> Option<&GpioSnapshot> {
        self.gpio.as_ref()
    }

    /// The latest audio meter snapshot.
    #[must_use]
    pub fn audio(&self) -> Option<&AudioMeterSnapshot> {
        self.audio.as_ref()
    }

    /// The latest batch of log lines.
    #[must_use]
    pub fn logs(&self) -> &[LogEntry] {
        &self.logs
    }

    /// The latest redacted config.
    #[must_use]
    pub fn config(&self) -> Option<&ConfigRedacted> {
        self.config.as_ref()
    }

    /// Whether the booth permits control (simulate) actions, per its config.
    #[must_use]
    pub fn controls_allowed(&self) -> bool {
        self.config
            .as_ref()
            .is_some_and(|config| config.debug.allow_controls)
    }

    /// Advance the log-level filter to the next level and re-poll.
    pub fn cycle_log_level(&mut self) {
        let next = LOG_LEVELS
            .iter()
            .position(|level| *level == self.log_level)
            .map_or(0, |index| (index + 1) % LOG_LEVELS.len());
        self.log_level = LOG_LEVELS[next].to_owned();
        self.refresh();
    }

    /// Trigger a poll unless one is already in flight.
    pub fn refresh(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let client = self.client.clone();
        let level = self.log_level.clone();
        tokio::spawn(async move {
            let (state, gpio, audio, logs, config) = tokio::join!(
                client.state(),
                client.gpio(),
                client.audio(),
                client.logs(Some(level.as_str()), Some(LOG_LIMIT)),
                client.config(),
            );
            let fetch = DebugFetch {
                state: state.map_err(|err| err.to_string()),
                gpio: gpio.map_err(|err| err.to_string()),
                audio: audio.map_err(|err| err.to_string()),
                logs: logs.map_err(|err| err.to_string()),
                config: config.map_err(|err| err.to_string()),
            };
            let _ = tx.send(fetch);
        });
    }

    /// Apply any completed poll (non-blocking). Called each tick.
    pub fn drain(&mut self) {
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(fetch) => self.apply(fetch),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    // The poll task ended without sending (panic/cancel). Clear
                    // the in-flight flag so the controller can recover.
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    /// Advance the controller: apply results, then auto-poll when the screen is
    /// `focused`. The panels are always seeded with one REST round (the config
    /// never arrives over telemetry); after that, polling continues on the
    /// interval only while the live socket is off — when live, telemetry drives
    /// the panels instead.
    pub fn tick(&mut self, focused: bool) {
        self.drain();
        self.drain_stream();
        if !focused {
            return;
        }
        let seeded = self.last_refresh.is_some();
        let should_seed = !seeded && !self.in_flight;
        let should_poll = seeded && !self.live && self.is_due(Instant::now());
        if should_seed || should_poll {
            self.refresh();
        }
    }

    /// Whether an auto-poll is due at `now`.
    fn is_due(&self, now: Instant) -> bool {
        if self.in_flight {
            return false;
        }
        self.last_refresh
            .is_none_or(|last| now.duration_since(last) >= POLL_INTERVAL)
    }

    /// Apply one polling round, keeping the previous value for any endpoint
    /// that failed and recording the failures for the status footer.
    fn apply(&mut self, fetch: DebugFetch) {
        self.in_flight = false;
        self.last_refresh = Some(Instant::now());
        self.rx = None;

        let mut errors = Vec::new();
        match fetch.state {
            Ok(value) => self.state = Some(value),
            Err(error) => errors.push(format!("state: {error}")),
        }
        match fetch.gpio {
            Ok(value) => self.gpio = Some(value),
            Err(error) => errors.push(format!("gpio: {error}")),
        }
        match fetch.audio {
            Ok(value) => self.audio = Some(value),
            Err(error) => errors.push(format!("audio: {error}")),
        }
        match fetch.logs {
            Ok(value) => self.logs = value,
            Err(error) => errors.push(format!("logs: {error}")),
        }
        match fetch.config {
            Ok(value) => self.config = Some(value),
            Err(error) => errors.push(format!("config: {error}")),
        }

        let now = Instant::now();
        if errors.is_empty() {
            self.last_ok = Some(now);
            self.last_error = None;
        } else {
            self.last_error = Some((errors.join("; "), now));
        }
    }

    /// Toggle the live telemetry subscription on or off.
    ///
    /// Starting spawns a background task that connects to the booth telemetry
    /// WebSocket (replaying from just after the most recent record id when
    /// reconnecting) and forwards each decoded record into the panels via
    /// [`tick`](Self::tick); stopping aborts that task. While live the REST poll
    /// is suspended.
    pub fn toggle_live(&mut self) {
        if self.live {
            self.stop_live();
            return;
        }
        self.live = true;
        self.live_error = None;
        let (tx, rx) = unbounded_channel();
        self.stream_rx = Some(rx);
        let base_url = self.base_url.clone();
        let token = self.token.clone();
        let pinned = self.pinned_sha256.clone();
        let replay_from = self.last_record_id.map(|id| id.saturating_add(1));
        let task = tokio::spawn(async move {
            match connect_telemetry(&base_url, token.as_deref(), pinned.as_deref(), replay_from)
                .await
            {
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

    /// Stop the live subscription and release the background streaming task.
    fn stop_live(&mut self) {
        self.live = false;
        if let Some(task) = self.stream_task.take() {
            task.abort();
        }
        self.stream_rx = None;
    }

    /// Drain any live telemetry records delivered since the last tick.
    fn drain_stream(&mut self) {
        loop {
            let Some(rx) = self.stream_rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(Ok(record)) => self.apply_record(record),
                Ok(Err(error)) => {
                    self.live_error = Some((error, Instant::now()));
                    self.stop_live();
                    return;
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.stop_live();
                    return;
                }
            }
        }
    }

    /// Fold one live telemetry record into the cached panel snapshots.
    fn apply_record(&mut self, record: TelemetryRecord) {
        self.last_record_id = Some(record.id);
        match record.event {
            TelemetryEvent::StateTransition { to, .. } => {
                let updated_at = system_time_rfc3339(record.ts);
                if let Some(state) = self.state.as_mut() {
                    state.state = to;
                    state.updated_at = updated_at;
                } else {
                    self.state = Some(StatusSnapshot {
                        state: to,
                        updated_at,
                        current_question_id: None,
                        current_message_id: None,
                        last_error: None,
                    });
                }
            }
            TelemetryEvent::GpioEdge(edge) => self.apply_gpio_edge(&edge, record.id),
            TelemetryEvent::AudioLevel(level) => self.apply_audio_level(&level),
            TelemetryEvent::AudioDeviceChange { name, .. } => {
                if let Some(audio) = self.audio.as_mut() {
                    audio.current_device = Some(name);
                }
            }
            TelemetryEvent::Log {
                level,
                target,
                message,
            } => self.push_live_log(record.ts, level, target, message),
            TelemetryEvent::Error { source, message } => {
                self.push_live_log(record.ts, "error".to_owned(), source, message);
            }
            _ => {}
        }
    }

    /// Apply a GPIO edge to the cached snapshot, updating the matching pin or
    /// inserting a new one.
    fn apply_gpio_edge(&mut self, edge: &GpioEdge, record_id: u64) {
        let gpio = self.gpio.get_or_insert_with(GpioSnapshot::default);
        if let Some(pin) = gpio.pins.iter_mut().find(|pin| pin.role == edge.role) {
            pin.level = edge.level;
            pin.debounced_state = edge.level;
            pin.last_edge_monotonic_ns = edge.at_monotonic_ns;
            pin.last_event_id = record_id;
        } else {
            gpio.pins.push(GpioPinSnapshot {
                role: edge.role.clone(),
                level: edge.level,
                debounced_state: edge.level,
                last_edge_monotonic_ns: edge.at_monotonic_ns,
                last_event_id: record_id,
            });
        }
    }

    /// Apply an audio level-meter sample to the cached snapshot, converting the
    /// linear `[0,1]` magnitudes to dBFS.
    fn apply_audio_level(&mut self, level: &AudioLevel) {
        let audio = self.audio.get_or_insert_with(silent_audio_snapshot);
        let level_dbfs = linear_to_dbfs(level.rms);
        let peak_dbfs = linear_to_dbfs(level.peak);
        if level.channel == "output" {
            audio.output_level_dbfs = level_dbfs;
            audio.output_peak_dbfs = peak_dbfs;
        } else {
            audio.input_level_dbfs = level_dbfs;
            audio.input_peak_dbfs = peak_dbfs;
        }
    }

    /// Prepend a live log line, capping the buffer at [`LOG_LIMIT`].
    fn push_live_log(&mut self, ts: SystemTime, level: String, target: String, message: String) {
        self.logs.insert(
            0,
            LogEntry {
                ts: system_time_rfc3339(ts),
                level,
                target,
                message,
            },
        );
        self.logs.truncate(LOG_LIMIT);
    }

    /// Await and apply the next pending result (test helper).
    #[cfg(test)]
    async fn recv_once(&mut self) {
        if let Some(rx) = self.rx.as_mut()
            && let Some(fetch) = rx.recv().await
        {
            self.apply(fetch);
        }
    }
}

/// Convert a linear sample magnitude in `[0, 1]` to dBFS, flooring at `-120`.
fn linear_to_dbfs(magnitude: f32) -> f32 {
    if magnitude <= 0.0 {
        return -120.0;
    }
    (20.0 * magnitude.log10()).clamp(-120.0, 0.0)
}

/// A silent audio snapshot (all channels at the `-120` dBFS floor) used to seed
/// the cache before the first REST poll when telemetry arrives first.
fn silent_audio_snapshot() -> AudioMeterSnapshot {
    AudioMeterSnapshot {
        input_level_dbfs: -120.0,
        output_level_dbfs: -120.0,
        input_peak_dbfs: -120.0,
        output_peak_dbfs: -120.0,
        current_device: None,
        sample_rate_hz: None,
        updated_at: None,
    }
}

/// Format a [`SystemTime`] as an RFC3339 string, falling back to `unknown`.
fn system_time_rfc3339(ts: SystemTime) -> String {
    OffsetDateTime::from(ts)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use std::future::Future;
    use std::sync::{Arc, Mutex};

    use tbo_booth_client::{BoothTransport, HttpResponse, Result};

    use super::*;

    /// A transport that routes canned `200` bodies by request path so a single
    /// poll round can satisfy the state/gpio/audio/logs/config calls, with an
    /// optional set of paths forced to fail.
    #[derive(Clone, Default)]
    struct RoutingTransport {
        fail: Arc<Mutex<Vec<String>>>,
    }

    impl RoutingTransport {
        fn fail_path(path: &str) -> Self {
            Self {
                fail: Arc::new(Mutex::new(vec![path.to_owned()])),
            }
        }

        fn body_for(path: &str) -> &'static str {
            match path {
                "/v1/state" => {
                    r#"{"state":"idle","updatedAt":"2024-01-01T00:00:00Z","lastError":null}"#
                }
                "/v1/gpio" => r#"{"pins":[],"updatedAt":null}"#,
                "/v1/audio" => {
                    r#"{"inputLevelDbfs":-20.0,"outputLevelDbfs":-18.0,"inputPeakDbfs":-10.0,"outputPeakDbfs":-9.0}"#
                }
                "/v1/logs" => {
                    r#"[{"ts":"2024-01-01T00:00:00Z","level":"info","target":"booth","message":"hello"}]"#
                }
                "/v1/config" => {
                    r#"{"gpio":{},"audio":{},"operator":{"token":"…ab"},"debug":{"tailscaleEnabled":true,"lanEnabled":true,"allowControls":true,"runtimeMode":"simulator","ringBufferCapacity":256,"loopbackSkipAuth":true}}"#
                }
                _ => "{}",
            }
        }
    }

    impl BoothTransport for RoutingTransport {
        fn get(
            &self,
            path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> impl Future<Output = Result<HttpResponse>> + Send {
            let fails = self.fail.lock().unwrap().iter().any(|p| p == path);
            let response = if fails {
                HttpResponse {
                    status: 500,
                    body: "boom".to_owned(),
                }
            } else {
                HttpResponse {
                    status: 200,
                    body: Self::body_for(path).to_owned(),
                }
            };
            async move { Ok(response) }
        }

        async fn post(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
            _json_body: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(HttpResponse {
                status: 200,
                body: r#"{"accepted":true,"injected":1}"#.to_owned(),
            })
        }
    }

    fn controller(transport: RoutingTransport) -> DebugController<RoutingTransport> {
        let client = BoothClient::with_transport(transport, Some("token".to_owned()));
        DebugController::new(
            "booth-1".to_owned(),
            "http://127.0.0.1:8080".to_owned(),
            Some("token".to_owned()),
            None,
            client,
        )
    }

    fn record(id: u64, event: TelemetryEvent) -> TelemetryRecord {
        TelemetryRecord {
            id,
            ts: SystemTime::UNIX_EPOCH,
            event,
        }
    }

    #[tokio::test]
    async fn refresh_populates_all_panels() {
        let mut controller = controller(RoutingTransport::default());

        controller.refresh();
        controller.recv_once().await;

        assert!(!controller.is_refreshing());
        assert!(controller.last_error().is_none());
        assert!(controller.last_ok().is_some());
        assert_eq!(controller.state().unwrap().state, "idle");
        assert_eq!(controller.gpio().unwrap().pins.len(), 0);
        assert_eq!(controller.logs().len(), 1);
        assert!(controller.controls_allowed());
        assert!(controller.audio().is_some());
        assert!(controller.config().is_some());
    }

    #[tokio::test]
    async fn partial_failure_records_error_but_keeps_other_panels() {
        let mut controller = controller(RoutingTransport::fail_path("/v1/gpio"));

        controller.refresh();
        controller.recv_once().await;

        assert!(controller.last_error().is_some());
        assert!(
            controller.last_error().unwrap().0.contains("gpio"),
            "error should name the failing endpoint"
        );
        // Other panels still populated despite the gpio failure.
        assert!(controller.state().is_some());
        assert!(controller.config().is_some());
        assert!(controller.gpio().is_none());
        assert!(controller.last_ok().is_none());
    }

    #[tokio::test]
    async fn failed_panel_keeps_prior_value_on_next_round() {
        let mut controller = controller(RoutingTransport::default());
        controller.refresh();
        controller.recv_once().await;
        assert!(controller.gpio().is_some());

        // A subsequent round where gpio fails should keep the prior snapshot.
        let client = BoothClient::with_transport(RoutingTransport::fail_path("/v1/gpio"), None);
        controller.client = client;
        controller.refresh();
        controller.recv_once().await;

        assert!(controller.gpio().is_some(), "prior gpio value is retained");
        assert!(controller.last_error().is_some());
    }

    #[tokio::test]
    async fn cycle_log_level_advances_and_triggers_refresh() {
        let mut controller = controller(RoutingTransport::default());
        assert_eq!(controller.log_level(), "info");

        controller.cycle_log_level();
        assert_eq!(controller.log_level(), "debug");
        assert!(controller.is_refreshing());
        controller.recv_once().await;

        controller.cycle_log_level();
        assert_eq!(controller.log_level(), "trace");
        controller.recv_once().await;

        controller.cycle_log_level();
        assert_eq!(controller.log_level(), "error");
    }

    #[test]
    fn is_due_respects_interval_and_in_flight() {
        let mut controller = controller(RoutingTransport::default());
        let now = Instant::now();

        assert!(controller.is_due(now));

        controller.last_refresh = Some(now);
        assert!(!controller.is_due(now));
        assert!(controller.is_due(now + POLL_INTERVAL));

        controller.in_flight = true;
        assert!(!controller.is_due(now + POLL_INTERVAL));
    }

    #[test]
    fn live_state_transition_updates_state_panel() {
        let mut controller = controller(RoutingTransport::default());
        controller.apply_record(record(
            10,
            TelemetryEvent::StateTransition {
                from: "idle".to_owned(),
                to: "dialing".to_owned(),
                cause: "hook_off".to_owned(),
                at_monotonic_ns: 1,
            },
        ));
        assert_eq!(controller.state().unwrap().state, "dialing");
        assert_eq!(controller.last_record_id, Some(10));
    }

    #[test]
    fn live_gpio_edge_inserts_then_updates_pin() {
        let mut controller = controller(RoutingTransport::default());
        controller.apply_record(record(
            1,
            TelemetryEvent::GpioEdge(GpioEdge {
                role: "hook".to_owned(),
                level: true,
                at_monotonic_ns: 5,
            }),
        ));
        assert_eq!(controller.gpio().unwrap().pins.len(), 1);
        assert!(controller.gpio().unwrap().pins[0].level);

        controller.apply_record(record(
            2,
            TelemetryEvent::GpioEdge(GpioEdge {
                role: "hook".to_owned(),
                level: false,
                at_monotonic_ns: 9,
            }),
        ));
        let pins = &controller.gpio().unwrap().pins;
        assert_eq!(pins.len(), 1, "same role updates in place");
        assert!(!pins[0].level);
        assert_eq!(pins[0].last_event_id, 2);
    }

    #[test]
    fn live_audio_level_updates_only_its_channel() {
        let mut controller = controller(RoutingTransport::default());
        controller.apply_record(record(
            1,
            TelemetryEvent::AudioLevel(AudioLevel {
                channel: "output".to_owned(),
                peak: 1.0,
                rms: 1.0,
                at_monotonic_ns: 1,
            }),
        ));
        let audio = controller.audio().unwrap();
        assert!((audio.output_level_dbfs - 0.0).abs() < 1e-3);
        // The untouched input channel stays at the silent floor.
        assert!(audio.input_level_dbfs <= -120.0);
    }

    #[test]
    fn live_log_event_prepends_to_logs() {
        let mut controller = controller(RoutingTransport::default());
        controller.apply_record(record(
            1,
            TelemetryEvent::Log {
                level: "warn".to_owned(),
                target: "booth".to_owned(),
                message: "hi".to_owned(),
            },
        ));
        let first = controller.logs().first().unwrap();
        assert_eq!(first.message, "hi");
        assert_eq!(first.level, "warn");
    }

    #[test]
    fn linear_to_dbfs_maps_endpoints() {
        assert!((linear_to_dbfs(1.0) - 0.0).abs() < 1e-3);
        assert!(linear_to_dbfs(0.0) <= -120.0);
        assert!(linear_to_dbfs(-1.0) <= -120.0);
    }

    #[tokio::test]
    async fn toggle_live_sets_and_clears_flag() {
        let mut controller = controller(RoutingTransport::default());
        assert!(!controller.is_live());

        controller.toggle_live();
        assert!(controller.is_live());

        controller.toggle_live();
        assert!(!controller.is_live());
    }

    #[tokio::test]
    async fn stream_error_records_live_error_and_stops() {
        let mut controller = controller(RoutingTransport::default());
        controller.live = true;
        let (tx, rx) = unbounded_channel();
        controller.stream_rx = Some(rx);
        tx.send(Err("boom".to_owned())).unwrap();

        controller.drain_stream();

        assert!(!controller.is_live());
        assert!(controller.live_error().is_some());
        assert!(controller.live_error().unwrap().0.contains("boom"));
    }
}
