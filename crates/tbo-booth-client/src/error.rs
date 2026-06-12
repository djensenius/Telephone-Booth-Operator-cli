//! Error type for the booth debug-server client.

use serde::{Deserialize, Serialize};
use tbo_core::domain::RuntimeMode;

/// The `403 controls_denied` body returned when the booth refuses a simulate
/// request (e.g. the runtime is in `real` mode or controls are disabled).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlsDenied {
    /// Machine-readable error code; always `"controls_denied"`.
    pub error: String,
    /// Short reason the controls were denied.
    pub reason: String,
    /// Human-readable detail.
    pub detail: String,
    /// The booth's current runtime mode.
    pub runtime_mode: RuntimeMode,
}

/// Errors returned by [`BoothClient`](crate::BoothClient) calls.
#[derive(Debug, thiserror::Error)]
pub enum BoothError {
    /// The HTTP layer failed (DNS, TLS, connection, or body read).
    #[error("transport error: {0}")]
    Transport(String),
    /// The server rejected the debug token (`401`) or denied access (`403`)
    /// for a non-control reason.
    #[error("unauthorized (HTTP {0})")]
    Unauthorized(u16),
    /// The booth refused a control action (`403` with a `controls_denied`
    /// body), typically because the runtime is not in a simulatable mode.
    #[error("controls denied: {} ({})", .0.reason, .0.detail)]
    ControlsDenied(ControlsDenied),
    /// The requested resource does not exist (`404`).
    #[error("not found")]
    NotFound,
    /// The server returned an otherwise-unhandled non-success status.
    #[error("booth debug server returned HTTP {status}: {body}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Response body, for diagnostics.
        body: String,
    },
    /// The response body could not be decoded into the expected type.
    #[error("failed to decode response: {0}")]
    Decode(String),
    /// A request could not be constructed or serialized.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

/// A specialized [`Result`](std::result::Result) for booth client calls.
pub type Result<T> = std::result::Result<T, BoothError>;
