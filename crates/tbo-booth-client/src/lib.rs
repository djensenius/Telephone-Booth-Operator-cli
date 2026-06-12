//! Booth debug-server client for the `tb-operator` console.
//!
//! Connects directly to a booth's on-device debug server: typed REST snapshots
//! (state, GPIO, audio, system, logs, config, cert fingerprint), the
//! Prometheus `/metrics` scrape (parsed via [`tbo_metrics`]), telemetry-event
//! history, the live telemetry WebSocket, and the simulate control endpoints.
//! The REST client is generic over a [`BoothTransport`] so it can be
//! unit-tested without a network, and the LAN endpoints are reached over a
//! fingerprint-pinned TLS transport (see [`tls`]).

pub mod client;
pub mod error;
pub mod model;
pub mod telemetry;
pub mod tls;
pub mod transport;

pub use client::BoothClient;
pub use error::{BoothError, ControlsDenied, Result};
pub use model::{
    AudioLevel, AudioMeterSnapshot, CertFingerprint, ConfigRedacted, DebugConfigRedacted, GpioEdge,
    GpioPinSnapshot, GpioSnapshot, HealthResponse, LogEntry, OperatorConfigRedacted,
    SimulateResponse, StatusSnapshot, TelemetryEvent, TelemetryRecord,
};
pub use telemetry::{TelemetryStream, connect_telemetry};
pub use tls::pinned_tls_config;
pub use transport::{BoothTransport, HttpResponse, ReqwestBoothTransport};
