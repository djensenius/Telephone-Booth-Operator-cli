//! Call sessions: one row per phone call, with an event timeline on detail.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::event::BoothEventRecord;

/// How a call ended (mirrors the booth's `CallOutcome`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    /// Hung up before dialing.
    HungUpBeforeDial,
    /// Hung up while a prompt was playing.
    HungUpDuringPrompt,
    /// Hung up mid-recording.
    HungUpDuringRecording,
    /// Hung up during upload.
    HungUpDuringUpload,
    /// Recording completed normally.
    RecordingCompleted,
    /// Recording failed.
    RecordingFailed,
    /// Upload failed.
    UploadFailed,
    /// Operator-side error.
    OperatorError,
    /// Call was aborted.
    Aborted,
}

/// A single call session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSession {
    /// Session id.
    pub id: String,
    /// Booth identifier.
    pub booth_id: String,
    /// Boot session id.
    pub boot_id: String,
    /// When the call started.
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    /// When the call ended, if it has.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub ended_at: Option<OffsetDateTime>,
    /// Digits dialed during the call.
    #[serde(default)]
    pub digits_dialed: Option<String>,
    /// Outcome, once known.
    #[serde(default)]
    pub outcome: Option<CallOutcome>,
    /// Recording produced by the call, if any.
    #[serde(default)]
    pub recording_id: Option<String>,
    /// Call duration in milliseconds.
    #[serde(default)]
    pub duration_ms: Option<i64>,
    /// Booth client version captured from `call_started`.
    #[serde(default)]
    pub version: Option<String>,
}

/// A page of call sessions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSessionList {
    /// Sessions, newest first.
    pub items: Vec<CallSession>,
    /// Opaque cursor for the next page, or `None` at the end.
    pub next_cursor: Option<String>,
}

/// A call session with its full event timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallSessionDetail {
    /// The session fields (flattened onto the same JSON object).
    #[serde(flatten)]
    pub session: CallSession,
    /// Events belonging to this session, in order.
    pub events: Vec<BoothEventRecord>,
}
