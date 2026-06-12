//! The persisted authentication session.

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::tokens::OidcTokens;

/// The subset of the token bundle that is persisted between runs.
///
/// Stored as JSON in the OS keychain. The `access_token` is short-lived; the
/// `refresh_token` is the long-lived secret used to mint new access tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSession {
    /// Bearer access token for the operator API.
    pub access_token: String,
    /// Refresh token used to obtain new access tokens, when one was issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// ID token, retained for displaying identity claims.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// Absolute expiry of the access token, when known.
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub expires_at: Option<OffsetDateTime>,
}

impl StoredSession {
    /// Build a session from a freshly issued token bundle, resolving the
    /// absolute expiry relative to `now`.
    #[must_use]
    pub fn from_tokens(tokens: &OidcTokens, now: OffsetDateTime) -> Self {
        Self {
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            id_token: tokens.id_token.clone(),
            expires_at: tokens.expires_at(now),
        }
    }

    /// Whether the access token is within `margin` of expiry (or has no known
    /// expiry, in which case a refresh is attempted defensively).
    #[must_use]
    pub fn is_expiring_soon(&self, now: OffsetDateTime, margin: Duration) -> bool {
        self.expires_at.is_none_or(|expiry| now + margin >= expiry)
    }

    /// Whether the access token has already expired (no grace margin), or has
    /// no known expiry (in which case it is treated as expired so a refresh is
    /// forced).
    #[must_use]
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at.is_none_or(|expiry| now >= expiry)
    }
}
