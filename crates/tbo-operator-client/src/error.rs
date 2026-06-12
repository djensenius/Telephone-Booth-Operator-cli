//! Error type for the operator API client.

/// Errors returned by [`OperatorClient`](crate::OperatorClient) calls.
#[derive(Debug, thiserror::Error)]
pub enum OperatorError {
    /// The HTTP layer failed (DNS, TLS, connection, or body read).
    #[error("transport error: {0}")]
    Transport(String),
    /// The request required authentication but no access token was available
    /// (the operator is signed out).
    #[error("not authenticated")]
    Unauthenticated,
    /// The server rejected the credentials (`401`) or denied access (`403`).
    #[error("unauthorized (HTTP {0})")]
    Unauthorized(u16),
    /// The requested resource does not exist (`404`).
    #[error("not found")]
    NotFound,
    /// The server returned an otherwise-unhandled non-success status.
    #[error("operator API returned HTTP {status}: {body}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Response body, for diagnostics.
        body: String,
    },
    /// The response body could not be decoded into the expected type.
    #[error("failed to decode response: {0}")]
    Decode(String),
    /// A request could not be constructed (e.g. an unformattable timestamp).
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

/// A specialized [`Result`](std::result::Result) for operator client calls.
pub type Result<T> = std::result::Result<T, OperatorError>;
