//! Small shared value types used across multiple API resources.

use serde::{Deserialize, Serialize};

/// Reference to an audio blob (a booth recording or question prompt).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioRef {
    /// Fetch URL for the audio. May be a short-lived SAS URL that expires.
    pub url: String,
    /// Lower-case hex SHA-256 of the audio bytes (64 chars).
    pub sha256: String,
    /// Duration in milliseconds, or `None` when the booth could not measure it.
    pub duration_ms: Option<i64>,
}
