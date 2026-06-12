//! On-disk console configuration.
//!
//! The configuration is stored as TOML under the platform config directory
//! (e.g. `~/.config/tb-operator/config.toml` on Linux,
//! `~/Library/Application Support/io.telephonebooth.tb-operator/config.toml`
//! on macOS). [`Config::load`] returns defaults when no file exists, so a
//! fresh install works out of the box against the production operator API.
//!
//! Secrets handling: the per-booth static `debug_token` is sensitive and is
//! kept out of this file by default — the onboarding flow stores it in a
//! separate secrets file under the platform data directory (see
//! [`crate::secrets`]) and merges it back in at startup via
//! [`Config::merge_secrets`]. An inline `debug-token` is still honoured for
//! backwards compatibility. The Authentik refresh token is never stored here —
//! it is kept in the OS keychain by the `tbo-auth` crate.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Production operator API base URL.
pub const DEFAULT_OPERATOR_BASE_URL: &str = "https://api.telephonebooth.io";
/// Authentik OIDC issuer (reuses the mobile operator app's public client).
pub const DEFAULT_OIDC_ISSUER: &str =
    "https://auth.fluxhaus.io/application/o/telephone-booth-operator-mobile";
/// Public, PKCE/device-code OIDC client id (shared with the mobile app).
pub const DEFAULT_OIDC_CLIENT_ID: &str = "x0M0MleMvCSCx8MqIE2jVoYe57nAhGymIG8azTEY";
/// OIDC scopes requested at sign-in.
pub const DEFAULT_OIDC_SCOPES: &str = "openid email profile offline_access";

/// Operator API connection settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct OperatorConfig {
    /// Base URL of the operator API.
    pub base_url: String,
}

impl Default for OperatorConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_OPERATOR_BASE_URL.to_owned(),
        }
    }
}

/// Authentik OIDC settings for the device-code flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct AuthConfig {
    /// OIDC issuer base URL (its `.well-known` document is discovered).
    pub issuer: String,
    /// Public client id.
    pub client_id: String,
    /// Space-separated scopes.
    pub scopes: String,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            issuer: DEFAULT_OIDC_ISSUER.to_owned(),
            client_id: DEFAULT_OIDC_CLIENT_ID.to_owned(),
            scopes: DEFAULT_OIDC_SCOPES.to_owned(),
        }
    }
}

/// Connection details for one booth's on-device debug server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct BoothConfig {
    /// Stable booth identifier (matches the API's `boothId`).
    pub id: String,
    /// Human-friendly display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Debug server base URL (Tailscale loopback `http://…:8080` or LAN TLS
    /// `https://…:8443`).
    pub debug_base_url: String,
    /// Static bearer token for the debug server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_token: Option<String>,
    /// Pinned SHA-256 of the debug server's TLS certificate (LAN TLS only),
    /// lower-case hex with no separators.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned_sha256: Option<String>,
}

/// UI / theming preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct UiConfig {
    /// Theme name (e.g. `bell-canada`).
    pub theme: String,
    /// Status/data poll interval in milliseconds.
    pub poll_interval_ms: u64,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "bell-canada".to_owned(),
            poll_interval_ms: 5_000,
        }
    }
}

/// The full console configuration.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    /// Operator API settings.
    #[serde(default)]
    pub operator: OperatorConfig,
    /// Authentik OIDC settings.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Configured booths.
    #[serde(default)]
    pub booths: Vec<BoothConfig>,
    /// UI preferences.
    #[serde(default)]
    pub ui: UiConfig,
}

impl Config {
    /// Resolve the default config file path for this platform.
    pub fn default_path() -> Result<PathBuf> {
        let dirs = directories::ProjectDirs::from("io", "telephonebooth", "tb-operator")
            .ok_or(Error::NoConfigDir)?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load the configuration from the default path, returning defaults when
    /// the file does not exist.
    pub fn load() -> Result<Self> {
        let path = Self::default_path()?;
        Self::load_from(&path)
    }

    /// Load the configuration from a specific path, returning defaults when the
    /// file does not exist.
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

    /// Serialize the configuration to TOML.
    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Persist the configuration to the default path, creating parent
    /// directories as needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::default_path()?;
        self.save_to(&path)
    }

    /// Persist the configuration to a specific path, creating parent
    /// directories as needed.
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
        })
    }

    /// Look up a configured booth by id.
    #[must_use]
    pub fn booth(&self, id: &str) -> Option<&BoothConfig> {
        self.booths.iter().find(|b| b.id == id)
    }
}
