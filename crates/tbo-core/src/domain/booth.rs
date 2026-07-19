//! Booth runtime state and live status.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// The booth's call-flow state machine position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoothState {
    /// On-hook, waiting for a caller.
    Idle,
    /// Off-hook, playing dial tone.
    DialTone,
    /// Caller is dialing digits.
    Dialing,
    /// Playing the selected question prompt.
    PlayingQuestion,
    /// Beep that precedes recording.
    Beep,
    /// Recording the caller's message.
    Recording,
    /// Uploading a finished recording.
    Uploading,
    /// Playing back a stored message.
    PlayingMessage,
    /// Playing operator/system instructions.
    PlayingInstructions,
    /// Playing the "call cannot be completed" prompt (dials 3-9).
    CallUnavailable,
    /// Error state.
    Error,
}

/// How the booth is being driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeMode {
    /// Real hardware via `booth-pi` adapters.
    Real,
    /// In-memory `booth-mock` adapters.
    Mock,
    /// Interactive `ratatui` simulator.
    Simulator,
}

/// Live booth status as returned by `GET /v1/status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothStatus {
    /// Current call-flow state.
    pub state: BoothState,
    /// When this status was last updated (server clock).
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// Question currently being played, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_question_id: Option<String>,
    /// Message currently being played, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_message_id: Option<String>,
    /// Last error string, if the booth is in (or recently left) an error state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// How the booth is being driven, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<RuntimeMode>,
}

/// Status push payload (`updatedAt` optional; server stamps it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusUpdate {
    /// Current call-flow state.
    pub state: BoothState,
    /// When this update was produced, if the client supplied it.
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub updated_at: Option<OffsetDateTime>,
    /// Question currently being played, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_question_id: Option<String>,
    /// Message currently being played, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_message_id: Option<String>,
    /// Last error string, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// How the booth is being driven, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_mode: Option<RuntimeMode>,
}

/// Recent booth status snapshots for operator charts
/// (`GET /v1/status/history`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusHistory {
    /// Status snapshots, newest first.
    pub items: Vec<BoothStatus>,
}
