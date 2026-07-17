//! Backend data layer for the TUI.
//!
//! Bridges the `tbo-auth` session to the operator client via
//! [`SessionTokenProvider`] and hosts the per-screen controllers that fetch and
//! cache backend data off the UI thread — the operator API, a booth's
//! Prometheus `/metrics` scrape (System Health), and the booth debug-server
//! REST snapshots (Debug panel). Each controller mirrors the [`AuthController`](crate::auth::AuthController)
//! pattern: a background `tokio` task performs the request and the UI thread
//! applies the result on each tick via `drain`, so rendering never blocks on
//! the network.

mod debug;
mod events;
mod identity;
mod messages;
mod playback;
mod questions;
mod sessions;
mod stats;
mod status;
mod system;
mod system_health;
mod tokens;

use std::sync::Arc;
use std::time::Instant;

use time::OffsetDateTime;

use tbo_auth::{
    InMemoryTokenStore, KeyringTokenStore, ReqwestTransport as AuthTransport, SessionManager,
    TokenStore,
};
use tbo_core::config::Config;
use tbo_operator_client::{OperatorClient, OperatorError, ReqwestTransport, Result, TokenProvider};

pub use debug::DebugController;
pub use events::EventsController;
pub use identity::IdentityController;
pub use messages::MessagesController;
pub use playback::PlaybackController;
pub use questions::QuestionsController;
pub use sessions::SessionsController;
pub use stats::StatsController;
pub use status::StatusController;
pub use system::SystemController;
pub use system_health::SystemHealthController;
pub use tokens::TokensController;

/// The session manager shared between the auth controller and the data layer.
pub type SharedSession = Arc<SessionManager<Box<dyn TokenStore>, AuthTransport>>;

/// Build the shared session manager, preferring the OS keychain and falling
/// back to an ephemeral in-memory store when secure storage is unavailable
/// (e.g. a headless host with no secret service, or a locked keychain).
///
/// Returns the session plus an optional warning describing why the in-memory
/// fallback was used, so callers (the TUI or a headless command) can surface it
/// however suits them.
///
/// # Errors
/// Returns an error only when even the in-memory session manager cannot be
/// constructed from the configured auth settings.
pub fn build_shared_session(config: &Config) -> Result<(SharedSession, Option<String>)> {
    match keyring_session(config) {
        Ok(session) => Ok((session, None)),
        Err(err) => {
            let warning =
                format!("Secure storage unavailable ({err}); using an in-memory session this run.");
            let store: Box<dyn TokenStore> = Box::new(InMemoryTokenStore::new());
            let manager = SessionManager::new(&config.auth, store)
                .map_err(|err| OperatorError::Transport(err.to_string()))?;
            Ok((Arc::new(manager), Some(warning)))
        }
    }
}

/// Build a keychain-backed session manager, surfacing any keychain construction
/// or initial-load error so the in-memory fallback can engage.
fn keyring_session(config: &Config) -> Result<SharedSession> {
    let store: Box<dyn TokenStore> = Box::new(
        KeyringTokenStore::new().map_err(|err| OperatorError::Transport(err.to_string()))?,
    );
    let manager = SessionManager::new(&config.auth, store)
        .map_err(|err| OperatorError::Transport(err.to_string()))?;
    // Probe the store now so a keychain read failure triggers the fallback
    // rather than surfacing later on first use.
    manager
        .current_session()
        .map_err(|err| OperatorError::Transport(err.to_string()))?;
    Ok(Arc::new(manager))
}

/// The concrete operator client the TUI uses for all read endpoints.
pub type OperatorApi = OperatorClient<ReqwestTransport, SessionTokenProvider>;

/// The result of a completed operator write action, surfaced by the app as a
/// toast. Shared by the controllers that issue mutating requests (messages,
/// questions, …); the app reloads the relevant list when `ok` is `true`.
#[derive(Debug, Clone)]
pub struct ActionOutcome {
    /// Human-readable summary of the outcome.
    pub message: String,
    /// Whether the action succeeded.
    pub ok: bool,
}

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
