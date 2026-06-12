//! Questions: the prompts the booth plays to callers.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::common::AudioRef;

/// Publication state of a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionStatus {
    /// Not yet published.
    Draft,
    /// Eligible to be played by the booth.
    Active,
    /// Retired.
    Archived,
}

impl QuestionStatus {
    /// The query-string value the API expects (`draft`, `active`, `archived`).
    #[must_use]
    pub const fn as_query(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Active => "active",
            Self::Archived => "archived",
        }
    }
}

/// A question prompt with its recorded audio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Question {
    /// Question id.
    pub id: String,
    /// Prompt text (1..=280 chars).
    pub prompt: String,
    /// Publication status.
    pub status: QuestionStatus,
    /// When the question was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Recorded prompt audio.
    pub audio: AudioRef,
}

/// Body for `POST /v1/questions`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionCreate {
    /// Prompt text (1..=280 chars).
    pub prompt: String,
    /// Id of the uploaded audio file (from the SAS upload flow).
    pub audio_file_id: String,
    /// Initial status; defaults server-side when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<QuestionStatus>,
}

/// A page of questions (`GET /v1/questions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionList {
    /// Questions on this page, newest first.
    pub items: Vec<Question>,
    /// Opaque cursor for the next page, or `None` at the end.
    #[serde(default)]
    pub next_cursor: Option<String>,
}
