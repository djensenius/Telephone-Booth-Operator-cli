//! Sensitive values kept outside the TOML config file.
//!
//! The main [`Config`] lives in the platform *config*
//! directory (e.g. `~/.config/tb-operator/config.toml`) and is intended to be
//! human-editable and safe to share. Secrets such as a booth's static debug
//! bearer token are instead written to a separate file in the platform *data*
//! directory (e.g. `~/.local/share/tb-operator/secrets.toml`) with
//! owner-only permissions, so they never appear in the shareable config.
//!
//! The Authentik refresh token is handled separately again: it is stored in
//! the OS keychain by the `tbo-auth` crate and never touches disk here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{Error, Result};

/// Sensitive values stored apart from the shareable TOML config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Secrets {
    /// Per-booth debug-server bearer tokens, keyed by booth id.
    #[serde(default)]
    pub booth_tokens: BTreeMap<String, String>,
}

impl Secrets {
    /// Resolve the default secrets file path for this platform (under the data
    /// directory rather than the config directory).
    ///
    /// # Errors
    /// Returns [`Error::NoConfigDir`] when no platform data directory is known.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("io", "telephonebooth", "tb-operator")
            .ok_or(Error::NoConfigDir)?;
        Ok(dirs.data_dir().join("secrets.toml"))
    }

    /// Load the secrets from the default path, returning empty secrets when the
    /// file does not exist.
    ///
    /// # Errors
    /// Returns an error when the path cannot be resolved, the file cannot be
    /// read, or its contents are not valid TOML.
    pub fn load() -> Result<Self> {
        let path = Self::default_path()?;
        Self::load_from(&path)
    }

    /// Load the secrets from a specific path, returning empty secrets when the
    /// file does not exist.
    ///
    /// # Errors
    /// Returns an error when the file cannot be read or is not valid TOML.
    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(toml::from_str(&text)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Serialize the secrets to TOML.
    ///
    /// # Errors
    /// Returns an error when serialization fails.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Persist the secrets to the default path, creating parent directories as
    /// needed and restricting the file to the current user.
    ///
    /// # Errors
    /// Returns an error when the path cannot be resolved or the file cannot be
    /// written.
    pub fn save(&self) -> Result<()> {
        let path = Self::default_path()?;
        self.save_to(&path)
    }

    /// Persist the secrets to a specific path, creating parent directories as
    /// needed and restricting the file to the current user (mode `0600` on
    /// Unix).
    ///
    /// # Errors
    /// Returns an error when parent directories or the file cannot be written.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text = self.to_toml()?;
        std::fs::write(path, text).map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?;
        restrict_to_owner(path)
    }

    /// Whether any secrets are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.booth_tokens.is_empty()
    }

    /// Look up a booth's stored debug token.
    #[must_use]
    pub fn booth_token(&self, booth_id: &str) -> Option<&str> {
        self.booth_tokens.get(booth_id).map(String::as_str)
    }

    /// Store (or clear) a booth's debug token. An empty token removes any
    /// existing entry so secrets that are no longer needed are not retained.
    pub fn set_booth_token(&mut self, booth_id: impl Into<String>, token: impl Into<String>) {
        let token = token.into();
        if token.is_empty() {
            self.booth_tokens.remove(&booth_id.into());
        } else {
            self.booth_tokens.insert(booth_id.into(), token);
        }
    }
}

impl Config {
    /// Fill in per-booth debug tokens from a [`Secrets`] store.
    ///
    /// A token stored in `secrets` takes precedence over any inline token left
    /// in the config file, so the data-dir secrets file is the canonical store
    /// while older configs with inline `debug-token` values keep working.
    pub fn merge_secrets(&mut self, secrets: &Secrets) {
        for booth in &mut self.booths {
            if let Some(token) = secrets.booth_token(&booth.id) {
                booth.debug_token = Some(token.to_owned());
            }
        }
    }

    /// Split the per-booth debug tokens out of the config into a [`Secrets`]
    /// store, clearing the inline tokens so they are not written to the
    /// shareable config file.
    #[must_use]
    pub fn take_secrets(&mut self) -> Secrets {
        let mut secrets = Secrets::default();
        for booth in &mut self.booths {
            if let Some(token) = booth.debug_token.take() {
                secrets.set_booth_token(booth.id.clone(), token);
            }
        }
        secrets
    }
}

/// Restrict a file to owner read/write. A no-op on platforms without Unix
/// permission bits.
#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, permissions).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Restrict a file to owner read/write. A no-op on platforms without Unix
/// permission bits.
#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<()> {
    Ok(())
}
