//! Token and device-authorization payloads exchanged with Authentik.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Response of the OAuth 2.0 device authorization request (RFC 8628),
/// decoded directly from Authentik's `/device/` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAuthorization {
    /// Opaque code the client polls the token endpoint with.
    pub device_code: String,
    /// Short code the user enters at the verification URL.
    pub user_code: String,
    /// URL the user visits to enter the `user_code`.
    pub verification_uri: String,
    /// URL that embeds the `user_code` for one-tap / QR verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    /// Lifetime of the `device_code` in seconds.
    pub expires_in: i64,
    /// Minimum interval, in seconds, between token-endpoint polls.
    #[serde(default = "default_interval")]
    pub interval: i64,
}

const fn default_interval() -> i64 {
    5
}

/// Token bundle returned by Authentik's `/token/` endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OidcTokens {
    /// Bearer access token sent to the operator API.
    pub access_token: String,
    /// Refresh token, when the provider issues one (`offline_access`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// ID token, when the `openid` scope is granted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    /// Lifetime of the access token in seconds, when provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    /// Token type (typically `Bearer`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
}

impl OidcTokens {
    /// Compute the absolute expiry instant relative to `now`, when the
    /// provider reported an `expires_in`.
    #[must_use]
    pub fn expires_at(&self, now: OffsetDateTime) -> Option<OffsetDateTime> {
        self.expires_in
            .map(|secs| now + time::Duration::seconds(secs))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn device_authorization_decodes_authentik_shape() {
        let json = r#"{
            "device_code": "dc-123",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://auth.example/device",
            "verification_uri_complete": "https://auth.example/device?code=ABCD-EFGH",
            "expires_in": 600,
            "interval": 5
        }"#;
        let da: DeviceAuthorization = serde_json::from_str(json).unwrap();
        assert_eq!(da.device_code, "dc-123");
        assert_eq!(da.user_code, "ABCD-EFGH");
        assert_eq!(da.expires_in, 600);
        assert_eq!(da.interval, 5);
        assert!(da.verification_uri_complete.is_some());
    }

    #[test]
    fn device_authorization_defaults_interval() {
        let json = r#"{
            "device_code": "dc",
            "user_code": "U",
            "verification_uri": "https://auth.example/device",
            "expires_in": 300
        }"#;
        let da: DeviceAuthorization = serde_json::from_str(json).unwrap();
        assert_eq!(da.interval, 5);
        assert_eq!(da.verification_uri_complete, None);
    }

    #[test]
    fn tokens_decode_and_compute_expiry() {
        let json = r#"{
            "access_token": "at",
            "refresh_token": "rt",
            "expires_in": 300,
            "token_type": "Bearer"
        }"#;
        let tokens: OidcTokens = serde_json::from_str(json).unwrap();
        assert_eq!(tokens.access_token, "at");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));

        let now = OffsetDateTime::UNIX_EPOCH;
        let expiry = tokens.expires_at(now).unwrap();
        assert_eq!(expiry, now + time::Duration::seconds(300));
    }

    #[test]
    fn tokens_without_expiry_have_no_expiry_instant() {
        let json = r#"{ "access_token": "at" }"#;
        let tokens: OidcTokens = serde_json::from_str(json).unwrap();
        assert_eq!(tokens.refresh_token, None);
        assert_eq!(tokens.expires_at(OffsetDateTime::UNIX_EPOCH), None);
    }
}
