//! Usage statistics overview (`GET /v1/stats/overview`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::OffsetDateTime;

/// Stats time window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StatsWindow {
    /// Last 24 hours.
    #[serde(rename = "24h")]
    Day,
    /// Last 7 days.
    #[serde(rename = "7d")]
    Week,
    /// Last 30 days.
    #[serde(rename = "30d")]
    Month,
    /// All time.
    #[serde(rename = "all")]
    All,
}

impl StatsWindow {
    /// The query-string value the API expects (`24h`, `7d`, `30d`, `all`).
    #[must_use]
    pub const fn as_query(self) -> &'static str {
        match self {
            Self::Day => "24h",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::All => "all",
        }
    }
}

/// One day's call counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsCallsPerDay {
    /// Date in `YYYY-MM-DD` (UTC).
    pub date: String,
    /// Total calls that day.
    pub total: u64,
    /// Completed calls that day.
    pub completed: u64,
}

/// One hour-of-day bucket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsHourlyBucket {
    /// Hour of day, `0..=23` (UTC).
    pub hour: u8,
    /// Calls in that hour.
    pub calls: u64,
    /// Messages in that hour.
    pub messages: u64,
}

/// A frequently-answered question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsTopQuestion {
    /// Question id.
    pub question_id: String,
    /// Prompt text.
    pub prompt: String,
    /// Number of messages answering it.
    pub message_count: u64,
    /// When it was last used.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
    /// When it was retired, if applicable.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub retired_at: Option<OffsetDateTime>,
}

/// Per-booth activity breakdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsBoothBreakdown {
    /// Booth identifier.
    pub booth_id: String,
    /// Calls from that booth.
    pub calls: u64,
    /// Messages from that booth.
    #[serde(default)]
    pub messages: Option<u64>,
    /// When the booth was last seen.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_seen_at: Option<OffsetDateTime>,
}

/// Busiest hour and day-of-week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsBusiest {
    /// Busiest hour `0..=23`, if any activity.
    pub hour: Option<u8>,
    /// Busiest day-of-week `0..=6` (0 = Sunday).
    pub day_of_week: Option<u8>,
}

/// Call aggregates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsCalls {
    /// Total calls.
    pub total: u64,
    /// Completed calls.
    pub completed: u64,
    /// Calls currently in progress.
    pub in_progress: u64,
    /// Average call duration in milliseconds.
    pub average_duration_ms: Option<f64>,
    /// Longest call duration in milliseconds.
    pub longest_duration_ms: Option<f64>,
    /// Counts keyed by `CallOutcome` string (unknown keys rendered verbatim).
    pub outcomes: BTreeMap<String, u64>,
    /// Per-day call counts.
    pub per_day: Vec<StatsCallsPerDay>,
}

/// Message aggregates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsMessages {
    /// Total messages.
    pub total: u64,
    /// Counts keyed by `MessageStatus` string (unknown keys rendered verbatim).
    pub by_status: BTreeMap<String, u64>,
    /// Average recording duration in milliseconds.
    pub average_duration_ms: Option<f64>,
}

/// Playback aggregates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsPlayback {
    /// Number of message playbacks.
    pub total_playbacks: u64,
}

/// Pickup/hangup and dialed-digit aggregates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsPickupsHangups {
    /// Number of pickups.
    pub pickups: u64,
    /// Number of hangups.
    pub hangups: u64,
    /// Counts keyed by digit `"0".."9"`.
    pub digits_dialed: BTreeMap<String, u64>,
}

/// Upload success/failure aggregates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsUploads {
    /// Successful uploads.
    pub succeeded: u64,
    /// Failed uploads.
    pub failed: u64,
    /// Failure rate in `0.0..=1.0`, or `None` when there were no attempts.
    pub failure_rate: Option<f64>,
}

/// The full statistics overview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsOverview {
    /// Window the stats cover.
    pub window: StatsWindow,
    /// Inclusive range start, or `None` for "all".
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub range_start: Option<OffsetDateTime>,
    /// Range end.
    #[serde(with = "time::serde::rfc3339")]
    pub range_end: OffsetDateTime,
    /// When the overview was generated.
    #[serde(with = "time::serde::rfc3339")]
    pub generated_at: OffsetDateTime,
    /// Always `UTC`.
    pub timezone: String,
    /// Call aggregates.
    pub calls: StatsCalls,
    /// Message aggregates.
    pub messages: StatsMessages,
    /// Playback aggregates.
    pub playback: StatsPlayback,
    /// Pickup/hangup aggregates.
    pub pickups_hangups: StatsPickupsHangups,
    /// Upload aggregates.
    pub uploads: StatsUploads,
    /// Most-answered questions.
    pub top_questions: Vec<StatsTopQuestion>,
    /// Hour-of-day buckets.
    pub hourly: Vec<StatsHourlyBucket>,
    /// Busiest hour/day.
    pub busiest: StatsBusiest,
    /// When the last activity occurred.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_activity_at: Option<OffsetDateTime>,
    /// Per-booth breakdown.
    pub booth_breakdown: Vec<StatsBoothBreakdown>,
}
