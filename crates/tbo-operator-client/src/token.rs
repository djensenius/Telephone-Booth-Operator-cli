//! Access-token provisioning for authenticated requests.
//!
//! The client is decoupled from the authentication crate via [`TokenProvider`]:
//! the binary supplies an implementation backed by the `tbo-auth`
//! `SessionManager` (proactive refresh, keychain persistence), while tests use
//! [`StaticTokenProvider`]. Keeping the trait here avoids a dependency on
//! `tbo-auth` from this crate.

use std::future::Future;

use crate::error::Result;

/// Supplies the bearer access token for operator API requests.
///
/// Returning `Ok(None)` means the operator is signed out; the client surfaces
/// that as [`OperatorError::Unauthenticated`](crate::OperatorError::Unauthenticated)
/// for endpoints that require authentication.
pub trait TokenProvider: Send + Sync {
    /// Resolve the current access token, refreshing it if necessary.
    fn access_token(&self) -> impl Future<Output = Result<Option<String>>> + Send;
}

/// A [`TokenProvider`] that always yields the same (optional) token.
///
/// Useful for tests and for unauthenticated use of public endpoints.
#[derive(Debug, Clone, Default)]
pub struct StaticTokenProvider {
    token: Option<String>,
}

impl StaticTokenProvider {
    /// A provider that yields the given bearer token.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: Some(token.into()),
        }
    }

    /// A provider that yields no token (anonymous).
    #[must_use]
    pub fn anonymous() -> Self {
        Self { token: None }
    }
}

impl TokenProvider for StaticTokenProvider {
    async fn access_token(&self) -> Result<Option<String>> {
        Ok(self.token.clone())
    }
}
