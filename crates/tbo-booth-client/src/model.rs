//! Wire types for the booth debug server.
//!
//! These mirror the JSON shapes served by the `booth-debug` crate in the
//! Telephone-Booth monorepo. Casing differs by source: the debug-server DTOs
//! (`StatusSnapshot`, `GpioSnapshot`, …) and the system snapshot use
//! `camelCase`, while the telemetry event payloads (from `booth-hal`) are
//! `snake_case`. The system snapshot is reused from
//! [`tbo_core::domain::system::BoothSystemSnapshot`].
//!
//! Discriminator fields the booth models as small enums (`role`, `channel`,
//! call `outcome`) are kept as `String` here so a future booth that adds a
//! variant cannot break deserialization.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use tbo_core::domain::RuntimeMode;
use tbo_core::domain::system::BoothSystemSnapshot;

/// Response from `GET /healthz`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthResponse {
    /// Always `true` when the server is reachable.
    pub ok: bool,
    /// The debug server's crate version.
    pub version: String,
}

/// State object returned by `GET /v1/state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusSnapshot {
    /// Operator-compatible state name.
    pub state: String,
    /// RFC3339 timestamp when the state was observed.
    pub updated_at: String,
    /// Current question id, when known.
    #[serde(default)]
    pub current_question_id: Option<String>,
    /// Current message id, when known.
    #[serde(default)]
    pub current_message_id: Option<String>,
    /// Most recent error, when known.
    #[serde(default)]
    pub last_error: Option<String>,
}

/// Snapshot returned by `GET /v1/gpio`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpioSnapshot {
    /// Per-pin snapshots.
    pub pins: Vec<GpioPinSnapshot>,
    /// RFC3339 timestamp for the newest GPIO edge in the snapshot.
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// Per-pin GPIO snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GpioPinSnapshot {
    /// Logical role of the pin (`rotary_pulse`, `rotary_read`, or `hook`).
    pub role: String,
    /// Most recent debounced level.
    pub level: bool,
    /// Alias for `level`, kept explicit for UI clarity.
    pub debounced_state: bool,
    /// Runtime monotonic timestamp for the last edge, in nanoseconds.
    pub last_edge_monotonic_ns: u64,
    /// Telemetry record id that carried the last edge.
    pub last_event_id: u64,
}

/// Snapshot returned by `GET /v1/audio`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioMeterSnapshot {
    /// Latest input RMS level in dBFS (clamped to -120 when silent).
    pub input_level_dbfs: f32,
    /// Latest output RMS level in dBFS (clamped to -120 when silent).
    pub output_level_dbfs: f32,
    /// Latest input peak level in dBFS (clamped to -120 when silent).
    pub input_peak_dbfs: f32,
    /// Latest output peak level in dBFS (clamped to -120 when silent).
    pub output_peak_dbfs: f32,
    /// Most recently reported device name, when known.
    #[serde(default)]
    pub current_device: Option<String>,
    /// Configured sample rate, when known.
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
    /// RFC3339 timestamp for the newest audio event in the snapshot.
    #[serde(default)]
    pub updated_at: Option<String>,
}

/// One log line from `GET /v1/logs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    /// RFC3339 timestamp of the log line.
    pub ts: String,
    /// Tracing level (`error`, `warn`, `info`, `debug`, `trace`).
    pub level: String,
    /// Tracing target (module path).
    pub target: String,
    /// Rendered message.
    pub message: String,
}

/// Operator-connection settings within [`ConfigRedacted`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperatorConfigRedacted {
    /// Operator API base URL, when configured.
    #[serde(default)]
    pub base_url: Option<String>,
    /// Status topic, when configured.
    #[serde(default)]
    pub status_topic: Option<String>,
    /// Redacted operator token (only the last few characters are shown).
    pub token: String,
}

/// Debug-surface settings within [`ConfigRedacted`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugConfigRedacted {
    /// Whether the Tailscale loopback listener is enabled.
    pub tailscale_enabled: bool,
    /// Whether the LAN TLS listener is enabled.
    pub lan_enabled: bool,
    /// Whether control endpoints (simulate) are permitted.
    pub allow_controls: bool,
    /// The booth's runtime mode.
    pub runtime_mode: RuntimeMode,
    /// Telemetry replay ring-buffer capacity.
    pub ring_buffer_capacity: u64,
    /// Allowed operator origin for CORS, when configured.
    #[serde(default)]
    pub operator_origin: Option<String>,
    /// Whether the loopback listener skips bearer auth.
    pub loopback_skip_auth: bool,
}

/// Redacted configuration returned by `GET /v1/config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRedacted {
    /// GPIO configuration, as an opaque JSON object.
    pub gpio: serde_json::Value,
    /// Audio configuration, as an opaque JSON object.
    pub audio: serde_json::Value,
    /// Operator connection settings.
    pub operator: OperatorConfigRedacted,
    /// Debug-surface settings.
    pub debug: DebugConfigRedacted,
}

/// Certificate fingerprint returned by `GET /v1/cert/fingerprint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertFingerprint {
    /// Lowercase, colon-separated SHA-256 of the certificate DER bytes.
    pub sha256: String,
}

/// Response from the `POST /v1/simulate/*` control endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulateResponse {
    /// Whether the booth accepted the injection.
    pub accepted: bool,
    /// Number of events injected.
    pub injected: u32,
}

/// A telemetry event with the metadata assigned by the booth's telemetry bus.
///
/// Returned as an array by `GET /v1/events` and pushed as individual frames
/// over the `/v1/ws/telemetry` WebSocket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryRecord {
    /// Monotonic id; pass back as `since` (or `replay_from`) to catch up.
    pub id: u64,
    /// Wall-clock timestamp assigned when the event was published.
    pub ts: SystemTime,
    /// The structured event payload.
    #[serde(flatten)]
    pub event: TelemetryEvent,
}

/// A GPIO edge payload (`gpio_edge` telemetry event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpioEdge {
    /// Pin role (`rotary_pulse`, `rotary_read`, or `hook`).
    pub role: String,
    /// New logical level after debounce.
    pub level: bool,
    /// Nanoseconds since the runtime started.
    pub at_monotonic_ns: u64,
}

/// An audio level-meter payload (`audio_level` telemetry event).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioLevel {
    /// Which side of the audio path the reading came from (`input`/`output`).
    pub channel: String,
    /// Peak sample magnitude in `[0.0, 1.0]`.
    pub peak: f32,
    /// RMS sample magnitude in `[0.0, 1.0]`.
    pub rms: f32,
    /// Nanoseconds since the runtime started.
    pub at_monotonic_ns: u64,
}

/// A structured telemetry event published by the booth.
///
/// Internally tagged by the `kind` field (snake_case). Unrecognised kinds
/// deserialize to [`TelemetryEvent::Unknown`] so new booth event types do not
/// break the client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TelemetryEvent {
    /// A debounced GPIO edge.
    GpioEdge(GpioEdge),
    /// A fully-decoded rotary digit.
    DigitDialed {
        /// Digit value, 0..=9.
        digit: u8,
        /// Number of pulses that decoded into this digit.
        pulses: u8,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// The state machine moved between states.
    StateTransition {
        /// State name before the event.
        from: String,
        /// State name after the event.
        to: String,
        /// Cause (the event kind that triggered it).
        cause: String,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// A periodic audio level-meter sample.
    AudioLevel(AudioLevel),
    /// The audio device was (re)selected or changed.
    AudioDeviceChange {
        /// Human-readable device name (best effort).
        name: String,
        /// Which side of the audio path changed (`input`/`output`).
        channel: String,
    },
    /// An outbound request to the operator.
    OperatorRequest {
        /// Correlation id.
        id: String,
        /// Route label.
        route: String,
    },
    /// An inbound response from the operator.
    OperatorResponse {
        /// Correlation id of the matching request.
        id: String,
        /// HTTP status code returned.
        status: u16,
        /// Round-trip duration, milliseconds.
        duration_ms: u64,
    },
    /// A free-form structured log line.
    Log {
        /// Tracing level as a lowercase string.
        level: String,
        /// Tracing target (module path).
        target: String,
        /// Rendered message.
        message: String,
    },
    /// A recoverable error that did not propagate.
    Error {
        /// Where the error came from.
        source: String,
        /// Display-formatted error.
        message: String,
    },
    /// A live host-vitals snapshot.
    SystemSample {
        /// The captured system snapshot.
        snapshot: Box<BoothSystemSnapshot>,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// A call session started (receiver off hook).
    CallStarted {
        /// Session id minted by the runtime.
        session_id: String,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// A call session ended (receiver on hook or otherwise terminated).
    CallEnded {
        /// Matching session id from the preceding `CallStarted`.
        session_id: String,
        /// Terminal outcome of the call.
        outcome: String,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// Recording of the caller's answer began.
    RecordingStarted {
        /// Adapter-assigned recording id.
        id: String,
        /// Session this recording belongs to.
        session_id: String,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// Recording of the caller's answer finished.
    RecordingStopped {
        /// Adapter-assigned recording id.
        id: String,
        /// Session this recording belongs to.
        session_id: String,
        /// Recording length, milliseconds.
        duration_ms: u64,
        /// Recording file size, bytes.
        bytes: u64,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// Upload to the operator started.
    UploadStarted {
        /// Recording being uploaded.
        recording_id: String,
        /// Session this upload belongs to.
        session_id: String,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// Upload to the operator completed successfully.
    UploadCompleted {
        /// Recording that was uploaded.
        recording_id: String,
        /// Session this upload belongs to.
        session_id: String,
        /// Time spent uploading, milliseconds.
        duration_ms: u64,
        /// Bytes uploaded.
        bytes: u64,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// Upload to the operator failed terminally.
    UploadFailed {
        /// Recording that was being uploaded.
        recording_id: String,
        /// Session this upload belongs to.
        session_id: String,
        /// Display-formatted error.
        message: String,
        /// Nanoseconds since runtime start.
        at_monotonic_ns: u64,
    },
    /// A telemetry kind this client does not recognise.
    #[serde(other)]
    Unknown,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    #[test]
    fn decodes_gpio_edge_record_with_flattened_payload() {
        let json = r#"{
            "id": 7,
            "ts": { "secs_since_epoch": 1700000000, "nanos_since_epoch": 0 },
            "kind": "gpio_edge",
            "role": "hook",
            "level": true,
            "at_monotonic_ns": 12345
        }"#;
        let record: TelemetryRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.id, 7);
        assert_eq!(
            record.event,
            TelemetryEvent::GpioEdge(GpioEdge {
                role: "hook".to_owned(),
                level: true,
                at_monotonic_ns: 12345,
            })
        );
    }

    #[test]
    fn decodes_struct_variant_event() {
        let json = r#"{
            "id": 9,
            "ts": { "secs_since_epoch": 1, "nanos_since_epoch": 2 },
            "kind": "digit_dialed",
            "digit": 5,
            "pulses": 5,
            "at_monotonic_ns": 999
        }"#;
        let record: TelemetryRecord = serde_json::from_str(json).unwrap();
        assert_eq!(
            record.event,
            TelemetryEvent::DigitDialed {
                digit: 5,
                pulses: 5,
                at_monotonic_ns: 999,
            }
        );
    }

    #[test]
    fn unknown_kind_falls_back_without_error() {
        let json = r#"{
            "id": 1,
            "ts": { "secs_since_epoch": 0, "nanos_since_epoch": 0 },
            "kind": "brand_new_event_kind",
            "whatever": 42
        }"#;
        let record: TelemetryRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.event, TelemetryEvent::Unknown);
    }

    #[test]
    fn decodes_status_snapshot_camel_case() {
        let json = r#"{
            "state": "idle",
            "updatedAt": "2024-01-01T00:00:00Z",
            "currentQuestionId": "q1"
        }"#;
        let status: StatusSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(status.state, "idle");
        assert_eq!(status.current_question_id.as_deref(), Some("q1"));
        assert_eq!(status.current_message_id, None);
    }

    #[test]
    fn decodes_audio_meter_snapshot() {
        let json = r#"{
            "inputLevelDbfs": -20.5,
            "outputLevelDbfs": -10.0,
            "inputPeakDbfs": -3.0,
            "outputPeakDbfs": -1.0,
            "sampleRateHz": 48000
        }"#;
        let audio: AudioMeterSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(audio.input_level_dbfs, -20.5);
        assert_eq!(audio.sample_rate_hz, Some(48000));
        assert_eq!(audio.current_device, None);
    }

    #[test]
    fn decodes_cert_fingerprint() {
        let fp: CertFingerprint = serde_json::from_str(r#"{"sha256":"aa:bb:cc"}"#).unwrap();
        assert_eq!(fp.sha256, "aa:bb:cc");
    }
}
