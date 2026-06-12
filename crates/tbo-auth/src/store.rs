//! Persistent storage for the authentication session.
//!
//! The [`TokenStore`] trait abstracts where the session is kept so the refresh
//! orchestration can be tested with an in-memory store. Production uses
//! [`KeyringTokenStore`], which keeps the session in the operating system's
//! secret store via the [`keyring`] crate.

use std::sync::Mutex;

use crate::error::{AuthError, Result};
use crate::session::StoredSession;

/// Keychain service identifier (shared across the operator console's secrets).
const KEYRING_SERVICE: &str = "io.telephonebooth.tb-operator";
/// Keychain account under which the OIDC session JSON is stored.
const KEYRING_ACCOUNT: &str = "oidc-session";

/// Persists the authentication session across runs.
pub trait TokenStore: Send + Sync {
    /// Load the stored session, or `None` when signed out.
    fn load(&self) -> Result<Option<StoredSession>>;
    /// Persist (replacing any existing) session.
    fn save(&self, session: &StoredSession) -> Result<()>;
    /// Remove any stored session (sign out).
    fn clear(&self) -> Result<()>;
}

impl TokenStore for Box<dyn TokenStore> {
    fn load(&self) -> Result<Option<StoredSession>> {
        (**self).load()
    }

    fn save(&self, session: &StoredSession) -> Result<()> {
        (**self).save(session)
    }

    fn clear(&self) -> Result<()> {
        (**self).clear()
    }
}

/// An in-memory [`TokenStore`], primarily for tests and ephemeral sessions.
#[derive(Debug, Default)]
pub struct InMemoryTokenStore {
    inner: Mutex<Option<StoredSession>>,
}

impl InMemoryTokenStore {
    /// Create an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for InMemoryTokenStore {
    fn load(&self) -> Result<Option<StoredSession>> {
        let guard = self.inner.lock().map_err(|_| poisoned())?;
        Ok(guard.clone())
    }

    fn save(&self, session: &StoredSession) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| poisoned())?;
        *guard = Some(session.clone());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let mut guard = self.inner.lock().map_err(|_| poisoned())?;
        *guard = None;
        Ok(())
    }
}

fn poisoned() -> AuthError {
    AuthError::Storage("token store lock poisoned".to_owned())
}

/// A [`TokenStore`] backed by the operating system's secret store.
///
/// Uses the platform-native backend (Keychain on macOS, the Windows Credential
/// Manager on Windows, and the kernel key-retention service on Linux). The
/// session is serialized to JSON and kept under a single keychain entry.
pub struct KeyringTokenStore {
    entry: keyring::Entry,
}

impl KeyringTokenStore {
    /// Open (or lazily create) the keychain entry for the session.
    ///
    /// # Errors
    /// Returns [`AuthError::Storage`] when the platform keychain cannot be
    /// accessed.
    pub fn new() -> Result<Self> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
            .map_err(|err| AuthError::Storage(err.to_string()))?;
        Ok(Self { entry })
    }
}

impl TokenStore for KeyringTokenStore {
    fn load(&self) -> Result<Option<StoredSession>> {
        match self.entry.get_password() {
            Ok(json) => {
                let session = serde_json::from_str(&json)
                    .map_err(|err| AuthError::Decode(err.to_string()))?;
                Ok(Some(session))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(AuthError::Storage(err.to_string())),
        }
    }

    fn save(&self, session: &StoredSession) -> Result<()> {
        let json =
            serde_json::to_string(session).map_err(|err| AuthError::Decode(err.to_string()))?;
        self.entry
            .set_password(&json)
            .map_err(|err| AuthError::Storage(err.to_string()))
    }

    fn clear(&self) -> Result<()> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(AuthError::Storage(err.to_string())),
        }
    }
}
