//! Error and result types for the authentication layer.

/// Errors raised by the Authentik authentication flow.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AuthError {
    /// The device authorization request was rejected by the provider.
    #[error("device authorization request failed: {0}")]
    DeviceAuthorizationFailed(String),

    /// The user denied the authorization request.
    #[error("authorization was denied")]
    AccessDenied,

    /// The device code expired before the user completed authorization.
    #[error("the device code expired before authorization completed")]
    ExpiredToken,

    /// Exchanging the device code (or authorization code) for tokens failed
    /// with a non-recoverable provider error.
    #[error("token exchange failed: {0}")]
    TokenExchangeFailed(String),

    /// The refresh token was rejected (a `4xx` response); the session must be
    /// re-established interactively.
    #[error("the refresh token was rejected: {0}")]
    RefreshTokenInvalid(String),

    /// A transient failure (network error or `5xx`); the operation may be
    /// retried.
    #[error("transient authentication failure: {0}")]
    Transient(String),

    /// The HTTP transport could not be constructed or a request could not be
    /// sent.
    #[error("http transport error: {0}")]
    Transport(String),

    /// A response body could not be decoded into the expected shape.
    #[error("failed to decode response: {0}")]
    Decode(String),

    /// The token store (OS keychain) could not be read or written.
    #[error("token storage error: {0}")]
    Storage(String),

    /// The flow was cancelled by the caller.
    #[error("authentication was cancelled")]
    Cancelled,
}

/// Convenience result alias for the authentication layer.
pub type Result<T> = std::result::Result<T, AuthError>;
