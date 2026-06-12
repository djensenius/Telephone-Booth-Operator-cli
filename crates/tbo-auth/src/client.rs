//! The Authentik authentication client: device-code flow and token refresh.

use std::time::Duration;

use tbo_core::config::AuthConfig;
use tokio::time::Instant;

use crate::endpoints::Endpoints;
use crate::error::{AuthError, Result};
use crate::tokens::{DeviceAuthorization, OidcTokens};
use crate::transport::{HttpTransport, ReqwestTransport};

/// `grant_type` value for the device authorization grant (RFC 8628).
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Outcome of a single device-code token-endpoint poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTokenOutcome {
    /// Authorization completed; tokens were issued.
    Tokens(OidcTokens),
    /// The user has not yet completed authorization; keep polling.
    Pending,
    /// The client is polling too fast; increase the interval.
    SlowDown,
    /// The user denied the request.
    Denied,
    /// The device code expired.
    Expired,
    /// The provider returned another OAuth error.
    Error(String),
}

/// Authentik authentication client.
///
/// Generic over the HTTP [`HttpTransport`] so the flow can be tested without a
/// network. Construct the production client with [`AuthClient::new`].
#[derive(Debug, Clone)]
pub struct AuthClient<T: HttpTransport = ReqwestTransport> {
    transport: T,
    endpoints: Endpoints,
    client_id: String,
    scopes: String,
}

impl AuthClient<ReqwestTransport> {
    /// Build a client with a default rustls-backed transport from the given
    /// OIDC configuration.
    pub fn new(config: &AuthConfig) -> Result<Self> {
        Ok(Self::with_transport(ReqwestTransport::new()?, config))
    }
}

impl<T: HttpTransport> AuthClient<T> {
    /// Build a client over an explicit transport (used in tests).
    #[must_use]
    pub fn with_transport(transport: T, config: &AuthConfig) -> Self {
        Self {
            transport,
            endpoints: Endpoints::derive(&config.issuer),
            client_id: config.client_id.clone(),
            scopes: config.scopes.clone(),
        }
    }

    /// Begin a device authorization request, returning the `user_code`,
    /// verification URL, polling interval, and `device_code`.
    pub async fn begin_device_authorization(&self) -> Result<DeviceAuthorization> {
        let form = [
            ("client_id", self.client_id.as_str()),
            ("scope", self.scopes.as_str()),
        ];
        let response = self
            .transport
            .post_form(&self.endpoints.device, &form)
            .await?;
        if response.is_success() {
            serde_json::from_str(&response.body).map_err(|err| AuthError::Decode(err.to_string()))
        } else {
            Err(AuthError::DeviceAuthorizationFailed(response.body))
        }
    }

    /// Perform a single token-endpoint poll for the given `device_code`.
    ///
    /// Network failures surface as [`AuthError::Transient`] so callers can keep
    /// polling; provider responses (including OAuth errors) map to a
    /// [`DeviceTokenOutcome`].
    pub async fn exchange_device_code(&self, device_code: &str) -> Result<DeviceTokenOutcome> {
        let form = [
            ("grant_type", DEVICE_CODE_GRANT),
            ("device_code", device_code),
            ("client_id", self.client_id.as_str()),
        ];
        let response = match self.transport.post_form(&self.endpoints.token, &form).await {
            Ok(response) => response,
            Err(AuthError::Transport(message)) => return Err(AuthError::Transient(message)),
            Err(other) => return Err(other),
        };
        if response.is_success() {
            let tokens = serde_json::from_str(&response.body)
                .map_err(|err| AuthError::Decode(err.to_string()))?;
            return Ok(DeviceTokenOutcome::Tokens(tokens));
        }
        Ok(match parse_oauth_error(&response.body).as_str() {
            "authorization_pending" => DeviceTokenOutcome::Pending,
            "slow_down" => DeviceTokenOutcome::SlowDown,
            "access_denied" => DeviceTokenOutcome::Denied,
            "expired_token" => DeviceTokenOutcome::Expired,
            "" => DeviceTokenOutcome::Error("unknown".to_owned()),
            other => DeviceTokenOutcome::Error(other.to_owned()),
        })
    }

    /// Poll the token endpoint until the user completes authorization, the code
    /// expires, or the request is denied.
    ///
    /// Honors the server-requested `interval` and backs off on `slow_down`.
    /// Transient network errors are retried; the loop ends when the
    /// `device_code` lifetime elapses.
    pub async fn poll_for_token(&self, authorization: &DeviceAuthorization) -> Result<OidcTokens> {
        let mut interval = u64::try_from(authorization.interval.max(1)).unwrap_or(5);
        let lifetime = u64::try_from(authorization.expires_in.max(1)).unwrap_or(600);
        let deadline = Instant::now() + Duration::from_secs(lifetime);

        while Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            match self.exchange_device_code(&authorization.device_code).await {
                Ok(DeviceTokenOutcome::Tokens(tokens)) => return Ok(tokens),
                Ok(DeviceTokenOutcome::Pending) => {}
                Ok(DeviceTokenOutcome::SlowDown) => interval += 5,
                Ok(DeviceTokenOutcome::Denied) => return Err(AuthError::AccessDenied),
                Ok(DeviceTokenOutcome::Expired) => return Err(AuthError::ExpiredToken),
                Ok(DeviceTokenOutcome::Error(body)) => {
                    return Err(AuthError::TokenExchangeFailed(body));
                }
                Err(AuthError::Transient(message)) => {
                    tracing::warn!(error = %message, "transient error while polling for token");
                }
                Err(other) => return Err(other),
            }
        }
        Err(AuthError::ExpiredToken)
    }

    /// Exchange a refresh token for a fresh token bundle.
    ///
    /// A `4xx` response means the refresh token is no longer valid
    /// ([`AuthError::RefreshTokenInvalid`]); other failures are
    /// [`AuthError::Transient`].
    pub async fn refresh(&self, refresh_token: &str) -> Result<OidcTokens> {
        let form = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
            ("scope", self.scopes.as_str()),
        ];
        let response = match self.transport.post_form(&self.endpoints.token, &form).await {
            Ok(response) => response,
            Err(AuthError::Transport(message)) => return Err(AuthError::Transient(message)),
            Err(other) => return Err(other),
        };
        if response.is_success() {
            serde_json::from_str(&response.body).map_err(|err| AuthError::Decode(err.to_string()))
        } else if response.is_client_error() {
            Err(AuthError::RefreshTokenInvalid(response.body))
        } else {
            Err(AuthError::Transient(format!(
                "token endpoint returned {}",
                response.status
            )))
        }
    }
}

/// Extract the `error` field from an OAuth error response body, returning an
/// empty string when it is absent or unparseable.
fn parse_oauth_error(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default()
}
