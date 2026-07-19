//! AI transcription and moderation records attached to messages.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

/// Which provider produced an AI result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    /// OpenAI hosted models.
    Openai,
    /// On-device Mac companion app.
    MacApp,
    /// AI pipeline disabled.
    Disabled,
}

/// Lifecycle of a transcription or moderation job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionStatus {
    /// Job has not finished yet.
    Pending,
    /// Job finished successfully.
    Succeeded,
    /// Job failed.
    Failed,
}

/// Moderation recommendation for a message. Advisory only: a human operator
/// always makes the final approve/reject decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationRecommendation {
    /// Suggests the message looks safe to approve.
    Approve,
    /// Suggests the message needs closer human review.
    Review,
    /// Suggests the message should be rejected.
    Reject,
}

/// Transcription (and optional translation) of a message's audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcription {
    /// Transcription id.
    pub id: String,
    /// Owning message id.
    pub message_id: String,
    /// Provider that produced the transcription.
    pub provider: AiProvider,
    /// Model name, when known.
    pub model: Option<String>,
    /// Job status.
    pub status: TranscriptionStatus,
    /// Transcribed text, when available.
    pub text: Option<String>,
    /// Detected source language.
    pub language: Option<String>,
    /// Audio duration the provider measured, in milliseconds.
    pub duration_ms: Option<i64>,
    /// Provider latency in milliseconds.
    pub latency_ms: Option<i64>,
    /// Error string when the job failed.
    pub error: Option<String>,
    /// Operator who requested the (re)run, if any.
    pub requested_by_id: Option<String>,
    /// When the job was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the job completed.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
    /// Translation job status, when a translation was attempted.
    pub translation_status: Option<TranscriptionStatus>,
    /// Translated text, when available.
    pub translated_text: Option<String>,
    /// Target language of the translation.
    pub translated_language: Option<String>,
    /// Provider that produced the translation.
    pub translation_provider: Option<AiProvider>,
    /// Translation model name.
    pub translation_model: Option<String>,
    /// Translation error string.
    pub translation_error: Option<String>,
    /// Translation latency in milliseconds.
    pub translation_latency_ms: Option<i64>,
    /// When the translation completed.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub translation_completed_at: Option<OffsetDateTime>,
}

/// All transcription attempts for a message, newest first
/// (`GET /v1/messages/{id}/transcriptions`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionList {
    /// Transcription attempts, newest first.
    pub items: Vec<Transcription>,
}

/// Moderation result for a message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Moderation {
    /// Moderation id.
    pub id: String,
    /// Owning message id.
    pub message_id: String,
    /// Transcription the moderation was run against, if any.
    pub transcription_id: Option<String>,
    /// Provider that produced the moderation.
    pub provider: AiProvider,
    /// Model name, when known.
    pub model: Option<String>,
    /// Job status.
    pub status: TranscriptionStatus,
    /// Whether the content was flagged.
    pub flagged: Option<bool>,
    /// Recommended action.
    pub recommendation: Option<ModerationRecommendation>,
    /// Highest category score in `0.0..=1.0`.
    pub max_score: Option<f64>,
    /// Per-category scores keyed by category name.
    pub categories: Option<BTreeMap<String, f64>>,
    /// Human-readable summary of why the content was flagged.
    pub reason_summary: Option<String>,
    /// Provider latency in milliseconds.
    pub latency_ms: Option<i64>,
    /// Error string when the job failed.
    pub error: Option<String>,
    /// Operator who requested the (re)run, if any.
    pub requested_by_id: Option<String>,
    /// When the job was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the job completed.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}
