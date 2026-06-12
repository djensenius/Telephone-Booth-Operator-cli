//! Shared error and result types for the `tb-operator` core layer.

use std::path::PathBuf;

/// Errors raised by the core layer (configuration I/O and (de)serialization).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Filesystem I/O failed for the given path.
    #[error("i/o error for {path}: {source}")]
    Io {
        /// Path that was being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The platform did not expose a usable config directory.
    #[error("could not determine a configuration directory for this platform")]
    NoConfigDir,

    /// Failed to parse the on-disk TOML configuration.
    #[error("failed to parse configuration: {0}")]
    ConfigParse(#[from] toml::de::Error),

    /// Failed to serialize the configuration to TOML.
    #[error("failed to serialize configuration: {0}")]
    ConfigSerialize(#[from] toml::ser::Error),

    /// JSON (de)serialization failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Convenience result alias for the core layer.
pub type Result<T> = std::result::Result<T, Error>;
