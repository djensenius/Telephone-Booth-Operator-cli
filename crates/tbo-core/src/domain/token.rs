//! API tokens for programmatic access.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Body for `POST /v1/api-tokens`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApiTokenRequest {
    /// Human-readable token name (1..=64 chars).
    pub name: String,
    /// Lifetime in days; never expires when omitted (max 3650).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_days: Option<u32>,
}

/// An API token's metadata (the secret is never returned after creation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiToken {
    /// Token id.
    pub id: String,
    /// Token name.
    pub name: String,
    /// Last four characters of the secret.
    pub last4: String,
    /// When the token was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the token expires, if ever.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// When the token was last used, if ever.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub last_used_at: Option<OffsetDateTime>,
    /// When the token was revoked, if it has been.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub revoked_at: Option<OffsetDateTime>,
}

/// Response to token creation, including the one-time plaintext secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenCreated {
    /// Token id.
    pub id: String,
    /// Token name.
    pub name: String,
    /// Last four characters of the secret.
    pub last4: String,
    /// When the token was created.
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// When the token expires, if ever.
    #[serde(default, with = "time::serde::rfc3339::option")]
    pub expires_at: Option<OffsetDateTime>,
    /// The plaintext secret — shown only once.
    pub plaintext: String,
}

/// One bucket of API-token usage over time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTokenUsageBucket {
    /// Bucket date (server format).
    pub date: String,
    /// Number of requests in the bucket.
    pub count: u64,
}
