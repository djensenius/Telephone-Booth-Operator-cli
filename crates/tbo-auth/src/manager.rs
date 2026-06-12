//! Session orchestration: keeps a valid access token available, refreshing
//! proactively and persisting the result.

use time::{Duration, OffsetDateTime};

use tbo_core::config::AuthConfig;

use crate::client::AuthClient;
use crate::error::{AuthError, Result};
use crate::session::StoredSession;
use crate::store::TokenStore;
use crate::tokens::OidcTokens;
use crate::transport::{HttpTransport, ReqwestTransport};

/// Default proactive-refresh margin: refresh once the access token is within
/// this window of expiry.
const DEFAULT_REFRESH_MARGIN_SECONDS: i64 = 60;

/// Coordinates the authentication client with a [`TokenStore`].
///
/// Wraps an [`AuthClient`] and a store, ensuring a valid access token is
/// available: it refreshes proactively when the token is near expiry, retains
/// the existing refresh token if the provider does not rotate it, signs out on
/// a rejected refresh token, and falls back to a still-valid token on transient
/// refresh failures. Concurrent calls are serialized so at most one refresh is
/// in flight.
pub struct SessionManager<S: TokenStore, T: HttpTransport = ReqwestTransport> {
    client: AuthClient<T>,
    store: S,
    refresh_margin: Duration,
    refresh_lock: tokio::sync::Mutex<()>,
}

impl<S: TokenStore> SessionManager<S, ReqwestTransport> {
    /// Build a manager with a default rustls-backed client from `config`.
    ///
    /// # Errors
    /// Returns an error when the HTTP client cannot be constructed.
    pub fn new(config: &AuthConfig, store: S) -> Result<Self> {
        Ok(Self::with_client(AuthClient::new(config)?, store))
    }
}

impl<S: TokenStore, T: HttpTransport> SessionManager<S, T> {
    /// Build a manager over an explicit [`AuthClient`] (used in tests).
    pub fn with_client(client: AuthClient<T>, store: S) -> Self {
        Self {
            client,
            store,
            refresh_margin: Duration::seconds(DEFAULT_REFRESH_MARGIN_SECONDS),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Override the proactive-refresh margin (default 60s).
    #[must_use]
    pub fn with_refresh_margin(mut self, margin: Duration) -> Self {
        self.refresh_margin = margin;
        self
    }

    /// The underlying authentication client (e.g. to begin a device login).
    pub fn client(&self) -> &AuthClient<T> {
        &self.client
    }

    /// The currently stored session, if any.
    pub fn current_session(&self) -> Result<Option<StoredSession>> {
        self.store.load()
    }

    /// Whether a session is stored (does not validate expiry).
    pub fn is_signed_in(&self) -> Result<bool> {
        Ok(self.store.load()?.is_some())
    }

    /// Persist a freshly issued token bundle, returning the stored session.
    pub fn complete_login(
        &self,
        tokens: &OidcTokens,
        now: OffsetDateTime,
    ) -> Result<StoredSession> {
        let session = StoredSession::from_tokens(tokens, now);
        self.store.save(&session)?;
        Ok(session)
    }

    /// Discard the stored session.
    pub fn sign_out(&self) -> Result<()> {
        self.store.clear()
    }

    /// Return a usable access token, refreshing if it is near expiry.
    ///
    /// Returns `Ok(None)` when signed out (or when an expired token cannot be
    /// refreshed). A rejected refresh token clears the session and surfaces
    /// [`AuthError::RefreshTokenInvalid`]; a transient refresh failure returns
    /// the existing token while it remains valid.
    pub async fn access_token(&self, now: OffsetDateTime) -> Result<Option<String>> {
        let _guard = self.refresh_lock.lock().await;

        let Some(session) = self.store.load()? else {
            return Ok(None);
        };
        if !session.is_expiring_soon(now, self.refresh_margin) {
            return Ok(Some(session.access_token));
        }
        let Some(refresh_token) = session.refresh_token.clone() else {
            if session.is_expired(now) {
                self.store.clear()?;
                return Ok(None);
            }
            return Ok(Some(session.access_token));
        };

        match self.client.refresh(&refresh_token).await {
            Ok(tokens) => {
                let mut next = StoredSession::from_tokens(&tokens, now);
                if next.refresh_token.is_none() {
                    next.refresh_token = Some(refresh_token);
                }
                if next.id_token.is_none() {
                    next.id_token = session.id_token.clone();
                }
                self.store.save(&next)?;
                Ok(Some(next.access_token))
            }
            Err(AuthError::RefreshTokenInvalid(reason)) => {
                self.store.clear()?;
                Err(AuthError::RefreshTokenInvalid(reason))
            }
            Err(AuthError::Transient(reason)) => {
                if session.is_expired(now) {
                    Err(AuthError::Transient(reason))
                } else {
                    Ok(Some(session.access_token))
                }
            }
            Err(other) => Err(other),
        }
    }
}
