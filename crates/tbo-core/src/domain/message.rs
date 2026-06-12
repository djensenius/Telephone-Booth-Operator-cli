//! Caller messages and the operator moderation actions on them.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::ai::{Moderation, Transcription};
use super::common::AudioRef;

/// Moderation lifecycle of a caller message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    /// Audio is still uploading.
    Uploading,
    /// Audio received; AI pipeline not yet complete.
    Received,
    /// Awaiting an operator decision.
    Pending,
    /// Approved for playback.
    Approved,
    /// Rejected.
    Rejected,
}

/// A caller-recorded message with its latest AI artifacts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Message id.
    pub id: String,
    /// Current moderation status.
    pub status: MessageStatus,
    /// Question the caller was answering, if any.
    #[serde(default)]
    pub question_id: Option<String>,
    /// Free-form operator notes.
    #[serde(default)]
    pub notes: Option<String>,
    /// When the message row was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the audio finished uploading.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub received_at: Option<OffsetDateTime>,
    /// When an operator decided on the message.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub decided_at: Option<OffsetDateTime>,
    /// Operator who decided, if any.
    #[serde(default)]
    pub decided_by_id: Option<String>,
    /// Audio reference for the recording.
    pub audio: AudioRef,
    /// Most recent transcription, when present.
    #[serde(default)]
    pub latest_transcription: Option<Transcription>,
    /// Most recent moderation, when present.
    #[serde(default)]
    pub latest_moderation: Option<Moderation>,
}

/// The two operator decisions a message can receive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDecisionKind {
    /// Approve the message for playback.
    Approve,
    /// Reject the message.
    Reject,
}

/// Body for `POST /v1/messages/{id}/decision`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDecision {
    /// Approve or reject.
    pub decision: MessageDecisionKind,
    /// Optional operator notes (max 2000 chars server-side).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Body for submitting an operator-provided translation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSubmit {
    /// Translated text (1..=20000 chars server-side).
    pub translated_text: String,
    /// Target language label, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub translated_language: Option<String>,
}
