//! Operator API data layer for the TUI.
//!
//! Bridges the `tbo-auth` session to the operator client via
//! [`SessionTokenProvider`] and hosts the per-screen controllers that fetch and
//! cache operator data off the UI thread. Each controller mirrors the
//! [`AuthController`](crate::auth::AuthController) pattern: a background `tokio`
//! task performs the request and the UI thread applies the result on each tick
//! via `drain`, so rendering never blocks on the network.

mod messages;
mod status;

use std::sync::Arc;
use std::time::Instant;

use time::OffsetDateTime;

use tbo_auth::{ReqwestTransport as AuthTransport, SessionManager, TokenStore};
use tbo_operator_client::{OperatorClient, OperatorError, ReqwestTransport, Result, TokenProvider};

pub use messages::MessagesController;
pub use status::StatusController;

/// The session manager shared between the auth controller and the data layer.
pub type SharedSession = Arc<SessionManager<Box<dyn TokenStore>, AuthTransport>>;

/// The concrete operator client the TUI uses for all read endpoints.
pub type OperatorApi = OperatorClient<ReqwestTransport, SessionTokenProvider>;

/// Adapts the `tbo-auth` [`SessionManager`] to the operator client's
/// [`TokenProvider`], performing proactive refresh on each token request.
///
/// Returns `Ok(None)` when signed out (the client maps that to
/// [`OperatorError::Unauthenticated`] for endpoints that require auth); a
/// refresh or storage failure surfaces as [`OperatorError::Transport`] with the
/// underlying message preserved for display.
#[derive(Clone)]
pub struct SessionTokenProvider {
    session: SharedSession,
}

impl SessionTokenProvider {
    /// Wrap a shared session manager.
    #[must_use]
    pub fn new(session: SharedSession) -> Self {
        Self { session }
    }
}

impl TokenProvider for SessionTokenProvider {
    async fn access_token(&self) -> Result<Option<String>> {
        self.session
            .access_token(OffsetDateTime::now_utc())
            .await
            .map_err(|err| OperatorError::Transport(err.to_string()))
    }
}

/// The load state of a resource fetched from the operator API.
///
/// A reload after a successful fetch keeps the previous `Ready` value visible;
/// the controller tracks the in-flight flag separately so the UI can show a
/// subtle "refreshing" indicator without blanking the screen.
#[derive(Debug, Clone)]
pub enum Remote<T> {
    /// Not yet requested.
    Idle,
    /// A first load is in flight, with no previous value to show.
    Loading,
    /// A value was loaded successfully.
    Ready {
        /// The most recently loaded value.
        value: T,
        /// When the value was fetched (monotonic clock).
        fetched_at: Instant,
    },
    /// The most recent load failed.
    Failed {
        /// Human-readable error message.
        error: String,
        /// When the failure occurred (monotonic clock).
        at: Instant,
    },
}
