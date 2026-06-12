//! The booth debug-server client: typed REST snapshots, the Prometheus
//! `/metrics` scrape, and the simulate control endpoints over a
//! [`BoothTransport`].

use serde::Serialize;
use serde::de::DeserializeOwned;
use tbo_core::domain::system::BoothSystemSnapshot;
use tbo_metrics::BoothMetrics;

use crate::error::{BoothError, ControlsDenied, Result};
use crate::model::{
    AudioMeterSnapshot, CertFingerprint, ConfigRedacted, GpioSnapshot, HealthResponse, LogEntry,
    SimulateResponse, StatusSnapshot, TelemetryRecord,
};
use crate::transport::{BoothTransport, HttpResponse, ReqwestBoothTransport};

/// A typed client for a single booth's on-device debug server.
///
/// Generic over the [`BoothTransport`] so calls can be tested without a
/// network. The debug token is sent as a bearer header on every request; pass
/// `None` for a loopback booth configured without auth.
#[derive(Debug, Clone)]
pub struct BoothClient<T: BoothTransport = ReqwestBoothTransport> {
    transport: T,
    token: Option<String>,
}

impl BoothClient<ReqwestBoothTransport> {
    /// Build a client against `base_url` using the default reqwest transport.
    ///
    /// # Errors
    /// Returns [`BoothError::Transport`] when the HTTP client cannot be built.
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Result<Self> {
        Ok(Self::with_transport(
            ReqwestBoothTransport::new(base_url)?,
            token,
        ))
    }

    /// Build a client against a booth LAN HTTPS `base_url` that trusts the
    /// self-signed certificate by its pinned SHA-256 `fingerprint`.
    ///
    /// # Errors
    /// Returns [`BoothError::InvalidRequest`] when `fingerprint` is not a valid
    /// 32-byte hex digest, or [`BoothError::Transport`] when the HTTP client
    /// cannot be built.
    pub fn with_pinned_tls(
        base_url: impl Into<String>,
        token: Option<String>,
        fingerprint: &str,
    ) -> Result<Self> {
        Ok(Self::with_transport(
            ReqwestBoothTransport::pinned(base_url, fingerprint)?,
            token,
        ))
    }

    /// Build a client for a booth, selecting the transport automatically: when
    /// `base_url` is an `https://` endpoint and `pinned_sha256` is set, the
    /// self-signed booth certificate is pinned to that fingerprint (LAN TLS);
    /// otherwise a default client is used (loopback `http://`).
    ///
    /// Connecting to an `https://` booth without a pinned fingerprint falls
    /// through to default CA validation, which will reject the self-signed
    /// certificate — set `pinned_sha256` for LAN TLS.
    ///
    /// # Errors
    /// Returns [`BoothError::InvalidRequest`] when a provided fingerprint is
    /// invalid, or [`BoothError::Transport`] when the HTTP client cannot be
    /// built.
    pub fn connect(
        base_url: impl Into<String>,
        token: Option<String>,
        pinned_sha256: Option<&str>,
    ) -> Result<Self> {
        let base_url = base_url.into();
        let is_https = base_url
            .trim_start()
            .get(..6)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https:"));
        match pinned_sha256.map(str::trim).filter(|fp| !fp.is_empty()) {
            Some(fingerprint) if is_https => Self::with_pinned_tls(base_url, token, fingerprint),
            _ => Self::new(base_url, token),
        }
    }
}

impl<T: BoothTransport> BoothClient<T> {
    /// Build a client over a custom transport (used by tests and the
    /// fingerprint-pinned LAN transport).
    pub fn with_transport(transport: T, token: Option<String>) -> Self {
        Self { transport, token }
    }

    /// The bearer token to attach to requests, if any.
    fn bearer(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// `GET /healthz` — liveness probe.
    pub async fn health(&self) -> Result<HealthResponse> {
        self.get_json("/healthz", &[]).await
    }

    /// `GET /v1/state` — current state snapshot.
    pub async fn state(&self) -> Result<StatusSnapshot> {
        self.get_json("/v1/state", &[]).await
    }

    /// `GET /v1/events` — telemetry records, optionally only those with an id
    /// strictly greater than `since`.
    pub async fn events(&self, since: Option<u64>) -> Result<Vec<TelemetryRecord>> {
        let mut query = Vec::new();
        if let Some(since) = since {
            query.push(("since", since.to_string()));
        }
        self.get_json("/v1/events", &query).await
    }

    /// `GET /v1/gpio` — per-pin GPIO snapshot.
    pub async fn gpio(&self) -> Result<GpioSnapshot> {
        self.get_json("/v1/gpio", &[]).await
    }

    /// `GET /v1/audio` — audio level-meter snapshot.
    pub async fn audio(&self) -> Result<AudioMeterSnapshot> {
        self.get_json("/v1/audio", &[]).await
    }

    /// `GET /v1/system` — live host-vitals snapshot.
    ///
    /// Returns `Ok(None)` when the booth has not yet produced a system sample
    /// (the server replies `204 No Content`).
    pub async fn system(&self) -> Result<Option<BoothSystemSnapshot>> {
        let response = self.transport.get("/v1/system", &[], self.bearer()).await?;
        if response.status == 204 {
            return Ok(None);
        }
        decode_success(response).map(Some)
    }

    /// `GET /v1/logs` — recent log lines, optionally filtered by `level` and
    /// capped at `limit` entries.
    pub async fn logs(&self, level: Option<&str>, limit: Option<usize>) -> Result<Vec<LogEntry>> {
        let mut query = Vec::new();
        if let Some(level) = level {
            query.push(("level", level.to_owned()));
        }
        if let Some(limit) = limit {
            query.push(("limit", limit.to_string()));
        }
        self.get_json("/v1/logs", &query).await
    }

    /// `GET /v1/config` — redacted runtime configuration.
    pub async fn config(&self) -> Result<ConfigRedacted> {
        self.get_json("/v1/config", &[]).await
    }

    /// `GET /v1/cert/fingerprint` — the LAN TLS certificate fingerprint.
    ///
    /// Only reachable over the loopback front door; a LAN request receives
    /// `403`, surfaced as [`BoothError::Unauthorized`].
    pub async fn cert_fingerprint(&self) -> Result<CertFingerprint> {
        self.get_json("/v1/cert/fingerprint", &[]).await
    }

    /// `GET /metrics` — the raw Prometheus text exposition.
    pub async fn metrics_text(&self) -> Result<String> {
        let response = self.transport.get("/metrics", &[], self.bearer()).await?;
        if response.is_success() {
            Ok(response.body)
        } else {
            Err(status_error(response))
        }
    }

    /// `GET /metrics`, parsed into a [`BoothMetrics`] snapshot.
    pub async fn metrics(&self) -> Result<BoothMetrics> {
        Ok(BoothMetrics::parse(&self.metrics_text().await?))
    }

    /// `POST /v1/simulate/event` — inject an arbitrary core event.
    ///
    /// `event` is serialized to JSON as the request body (the booth expects a
    /// `booth_core::Event`, internally tagged by an `event` field).
    pub async fn simulate_event<E: Serialize + Sync>(&self, event: &E) -> Result<SimulateResponse> {
        let body = serde_json::to_string(event)
            .map_err(|err| BoothError::InvalidRequest(err.to_string()))?;
        self.post_json("/v1/simulate/event", Some(&body)).await
    }

    /// `POST /v1/simulate/pulse` — inject `count` rotary pulses followed by a
    /// tick.
    pub async fn simulate_pulse(&self, count: u8) -> Result<SimulateResponse> {
        let body = serde_json::json!({ "count": count }).to_string();
        self.post_json("/v1/simulate/pulse", Some(&body)).await
    }

    /// Issue a `GET` and decode a successful JSON body.
    async fn get_json<R: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<R> {
        let response = self.transport.get(path, query, self.bearer()).await?;
        decode_success(response)
    }

    /// Issue a `POST` and decode a successful JSON body.
    async fn post_json<R: DeserializeOwned>(
        &self,
        path: &str,
        json_body: Option<&str>,
    ) -> Result<R> {
        let response = self
            .transport
            .post(path, &[], self.bearer(), json_body)
            .await?;
        decode_success(response)
    }
}

/// Decode a `2xx` JSON body, or map a non-success status to a [`BoothError`].
fn decode_success<R: DeserializeOwned>(response: HttpResponse) -> Result<R> {
    if !response.is_success() {
        return Err(status_error(response));
    }
    serde_json::from_str(&response.body).map_err(|err| BoothError::Decode(err.to_string()))
}

/// Map a non-success [`HttpResponse`] to the most specific [`BoothError`].
fn status_error(response: HttpResponse) -> BoothError {
    match response.status {
        401 => BoothError::Unauthorized(401),
        403 => serde_json::from_str::<ControlsDenied>(&response.body)
            .map_or(BoothError::Unauthorized(403), BoothError::ControlsDenied),
        404 => BoothError::NotFound,
        status => BoothError::Http {
            status,
            body: response.body,
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use crate::model::TelemetryEvent;

    /// A request recorded by [`FakeTransport`].
    #[derive(Debug, Clone)]
    struct RecordedCall {
        method: &'static str,
        path: String,
        query: Vec<(String, String)>,
        bearer: Option<String>,
        body: Option<String>,
    }

    /// An in-memory transport returning canned responses and recording calls.
    #[derive(Default)]
    struct FakeTransport {
        responses: Mutex<VecDeque<HttpResponse>>,
        calls: Mutex<Vec<RecordedCall>>,
    }

    impl FakeTransport {
        fn with_responses(responses: Vec<HttpResponse>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn ok(body: impl Into<String>) -> HttpResponse {
            HttpResponse {
                status: 200,
                body: body.into(),
            }
        }

        fn record(&self, call: RecordedCall) {
            self.calls.lock().unwrap().push(call);
        }

        fn next_response(&self) -> HttpResponse {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("a canned response")
        }

        fn last_call(&self) -> RecordedCall {
            self.calls.lock().unwrap().last().cloned().expect("a call")
        }
    }

    impl BoothTransport for FakeTransport {
        async fn get(
            &self,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            self.record(RecordedCall {
                method: "GET",
                path: path.to_owned(),
                query: query
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
                bearer: bearer.map(ToOwned::to_owned),
                body: None,
            });
            Ok(self.next_response())
        }

        async fn post(
            &self,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
            json_body: Option<&str>,
        ) -> Result<HttpResponse> {
            self.record(RecordedCall {
                method: "POST",
                path: path.to_owned(),
                query: query
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
                bearer: bearer.map(ToOwned::to_owned),
                body: json_body.map(ToOwned::to_owned),
            });
            Ok(self.next_response())
        }
    }

    fn client(transport: FakeTransport) -> BoothClient<FakeTransport> {
        BoothClient::with_transport(transport, Some("debug-token".to_owned()))
    }

    #[tokio::test]
    async fn state_sends_bearer_and_decodes() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(
            r#"{"state":"idle","updatedAt":"2024-01-01T00:00:00Z"}"#,
        )]);
        let client = client(transport);
        let state = client.state().await.unwrap();
        assert_eq!(state.state, "idle");

        let call = client.transport.last_call();
        assert_eq!(call.method, "GET");
        assert_eq!(call.path, "/v1/state");
        assert_eq!(call.bearer.as_deref(), Some("debug-token"));
    }

    #[tokio::test]
    async fn events_encodes_since_query() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok("[]")]);
        let client = client(transport);
        let events = client.events(Some(42)).await.unwrap();
        assert!(events.is_empty());

        let call = client.transport.last_call();
        assert_eq!(call.path, "/v1/events");
        assert_eq!(call.query, vec![("since".to_owned(), "42".to_owned())]);
    }

    #[tokio::test]
    async fn events_omits_since_when_absent() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok("[]")]);
        let client = client(transport);
        client.events(None).await.unwrap();
        assert!(client.transport.last_call().query.is_empty());
    }

    #[tokio::test]
    async fn events_decodes_telemetry_records() {
        let body = r#"[
            {"id":1,"ts":{"secs_since_epoch":1,"nanos_since_epoch":0},
             "kind":"state_transition","from":"idle","to":"dialing",
             "cause":"hook","at_monotonic_ns":5}
        ]"#;
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(body)]);
        let records = client(transport).events(None).await.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].event,
            TelemetryEvent::StateTransition {
                from: "idle".to_owned(),
                to: "dialing".to_owned(),
                cause: "hook".to_owned(),
                at_monotonic_ns: 5,
            }
        );
    }

    #[tokio::test]
    async fn system_returns_none_on_no_content() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 204,
            body: String::new(),
        }]);
        let snapshot = client(transport).system().await.unwrap();
        assert!(snapshot.is_none());
    }

    #[tokio::test]
    async fn system_decodes_snapshot() {
        let body = r#"{"cpu":{"usageRatio":0.5},"uptimeSeconds":3600}"#;
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(body)]);
        let snapshot = client(transport).system().await.unwrap().unwrap();
        assert_eq!(snapshot.uptime_seconds, Some(3600.0));
        assert_eq!(snapshot.cpu.and_then(|c| c.usage_ratio), Some(0.5));
    }

    #[tokio::test]
    async fn logs_builds_level_and_limit_query() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok("[]")]);
        let client = client(transport);
        client.logs(Some("warn"), Some(50)).await.unwrap();
        let call = client.transport.last_call();
        assert_eq!(call.path, "/v1/logs");
        assert_eq!(
            call.query,
            vec![
                ("level".to_owned(), "warn".to_owned()),
                ("limit".to_owned(), "50".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn metrics_scrapes_and_parses() {
        let exposition = "booth_cpu_usage_ratio{booth_id=\"b\"} 0.75\n";
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(exposition)]);
        let client = client(transport);
        let metrics = client.metrics().await.unwrap();
        assert_eq!(metrics.cpu_usage_ratio, Some(0.75));
        assert_eq!(client.transport.last_call().path, "/metrics");
    }

    #[tokio::test]
    async fn simulate_pulse_posts_count_body() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(
            r#"{"accepted":true,"injected":4}"#,
        )]);
        let client = client(transport);
        let response = client.simulate_pulse(3).await.unwrap();
        assert!(response.accepted);
        assert_eq!(response.injected, 4);

        let call = client.transport.last_call();
        assert_eq!(call.method, "POST");
        assert_eq!(call.path, "/v1/simulate/pulse");
        assert_eq!(call.body.as_deref(), Some(r#"{"count":3}"#));
    }

    #[tokio::test]
    async fn simulate_event_serializes_body() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(
            r#"{"accepted":true,"injected":1}"#,
        )]);
        let client = client(transport);
        let event = serde_json::json!({ "event": "hook_off" });
        client.simulate_event(&event).await.unwrap();
        assert_eq!(
            client.transport.last_call().body.as_deref(),
            Some(r#"{"event":"hook_off"}"#)
        );
    }

    #[tokio::test]
    async fn controls_denied_maps_to_typed_error() {
        let body = r#"{"error":"controls_denied","reason":"real mode",
            "detail":"controls disabled","runtimeMode":"real"}"#;
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 403,
            body: body.to_owned(),
        }]);
        let err = client(transport).simulate_pulse(1).await.unwrap_err();
        match err {
            BoothError::ControlsDenied(denied) => {
                assert_eq!(denied.reason, "real mode");
            }
            other => panic!("expected ControlsDenied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn plain_forbidden_maps_to_unauthorized() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 403,
            body: "Forbidden".to_owned(),
        }]);
        let err = client(transport).cert_fingerprint().await.unwrap_err();
        assert!(matches!(err, BoothError::Unauthorized(403)));
    }

    #[tokio::test]
    async fn not_found_maps_to_not_found() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 404,
            body: String::new(),
        }]);
        let err = client(transport).state().await.unwrap_err();
        assert!(matches!(err, BoothError::NotFound));
    }

    #[tokio::test]
    async fn anonymous_client_sends_no_bearer() {
        let transport = FakeTransport::with_responses(vec![FakeTransport::ok(
            r#"{"ok":true,"version":"1.0.0"}"#,
        )]);
        let client = BoothClient::with_transport(transport, None);
        client.health().await.unwrap();
        assert_eq!(client.transport.last_call().bearer, None);
    }

    const VALID_FINGERPRINT: &str =
        "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    #[test]
    fn connect_builds_plain_client_for_loopback_http() {
        assert!(BoothClient::connect("http://127.0.0.1:8080", None, None).is_ok());
    }

    #[test]
    fn connect_pins_tls_for_https_with_fingerprint() {
        assert!(
            BoothClient::connect("https://booth.ts.net:8443", None, Some(VALID_FINGERPRINT))
                .is_ok()
        );
    }

    #[test]
    fn connect_ignores_fingerprint_for_http() {
        // A fingerprint with a plaintext URL should not attempt TLS pinning, so
        // even an invalid fingerprint string is harmless here.
        assert!(BoothClient::connect("http://127.0.0.1:8080", None, Some("not-hex")).is_ok());
    }

    #[test]
    fn connect_rejects_invalid_fingerprint_over_https() {
        let err =
            BoothClient::connect("https://booth.ts.net:8443", None, Some("not-hex")).unwrap_err();
        assert!(matches!(err, BoothError::InvalidRequest(_)));
    }
}
