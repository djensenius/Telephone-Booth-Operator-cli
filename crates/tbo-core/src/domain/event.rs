//! Booth event log entries and their server-stored records.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

/// Discriminator for booth telemetry events (the `type` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoothEventType {
    /// A call began (handset lifted).
    CallStarted,
    /// A call ended (handset replaced).
    CallEnded,
    /// A rotary digit was dialed.
    DigitDialed,
    /// The booth state machine transitioned.
    StateTransition,
    /// Recording started.
    RecordingStarted,
    /// Recording stopped.
    RecordingStopped,
    /// An upload began.
    UploadStarted,
    /// An upload finished successfully.
    UploadCompleted,
    /// An upload failed.
    UploadFailed,
    /// A GPIO edge was observed.
    GpioEdge,
    /// The active audio device changed.
    AudioDeviceChange,
    /// The booth requested operator attention.
    OperatorRequest,
    /// The operator responded.
    OperatorResponse,
    /// An error occurred.
    Error,
    /// A free-form log line.
    Log,
    /// A periodic system metrics sample.
    SystemSample,
}

/// A booth-emitted event (as sent to `POST /v1/events` or streamed via SSE).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothEvent {
    /// Booth-assigned idempotency id.
    pub event_id: String,
    /// Booth identifier.
    pub booth_id: String,
    /// Boot session id (changes on each booth restart).
    pub boot_id: String,
    /// Event discriminator.
    #[serde(rename = "type")]
    pub event_type: BoothEventType,
    /// When the event occurred (booth clock).
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    /// Call session this event belongs to, if any.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Recording this event relates to, if any.
    #[serde(default)]
    pub recording_id: Option<String>,
    /// Arbitrary event-specific JSON payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// Running booth client version, when reported.
    #[serde(default)]
    pub version: Option<String>,
}

/// A stored event row returned by `GET /v1/events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothEventRecord {
    /// Server row id.
    pub id: String,
    /// Booth-assigned idempotency id.
    pub event_id: String,
    /// Booth identifier.
    pub booth_id: String,
    /// Boot session id.
    pub boot_id: String,
    /// Event discriminator.
    #[serde(rename = "type")]
    pub event_type: BoothEventType,
    /// When the event occurred (booth clock).
    #[serde(with = "time::serde::rfc3339")]
    pub occurred_at: OffsetDateTime,
    /// When the server received the event.
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
    /// Call session this event belongs to, if any.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Recording this event relates to, if any.
    #[serde(default)]
    pub recording_id: Option<String>,
    /// Full event payload JSON.
    #[serde(default)]
    pub payload: Value,
    /// Running booth client version, when reported.
    #[serde(default)]
    pub version: Option<String>,
}

/// A page of event records.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothEventList {
    /// Event rows, newest first.
    pub items: Vec<BoothEventRecord>,
    /// Opaque cursor for the next page, or `None` at the end.
    pub next_cursor: Option<String>,
}
