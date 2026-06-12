//! Live system snapshot uploaded by the booth (`GET /v1/system/current`).
//!
//! Every top-level field is optional so the type tolerates host adapters that
//! can only fill a subset, and stays forward-compatible with new metrics.

use serde::{Deserialize, Serialize};

use super::booth::RuntimeMode;

/// CPU utilization and load averages.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothCpuStats {
    /// Aggregate usage ratio in `0.0..=1.0`.
    #[serde(default)]
    pub usage_ratio: Option<f64>,
    /// Per-core usage ratios.
    #[serde(default)]
    pub per_core_usage_ratio: Option<Vec<f64>>,
    /// Number of physical cores.
    #[serde(default)]
    pub physical_cores: Option<u32>,
    /// 1-minute load average.
    #[serde(default)]
    pub load_avg1m: Option<f64>,
    /// 5-minute load average.
    #[serde(default)]
    pub load_avg5m: Option<f64>,
    /// 15-minute load average.
    #[serde(default)]
    pub load_avg15m: Option<f64>,
}

/// Memory and swap usage.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothMemoryStats {
    /// Total RAM in bytes.
    #[serde(default)]
    pub total_bytes: Option<u64>,
    /// Used RAM in bytes.
    #[serde(default)]
    pub used_bytes: Option<u64>,
    /// Total swap in bytes.
    #[serde(default)]
    pub swap_total_bytes: Option<u64>,
    /// Used swap in bytes.
    #[serde(default)]
    pub swap_used_bytes: Option<u64>,
}

/// Per-mount disk usage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothDiskStats {
    /// Mount point path.
    pub mount_point: String,
    /// Filesystem type, when known.
    #[serde(default)]
    pub filesystem: Option<String>,
    /// Total size in bytes.
    pub total_bytes: u64,
    /// Available bytes.
    pub available_bytes: u64,
}

/// Per-interface network counters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothNetworkStats {
    /// Interface name.
    pub interface: String,
    /// Cumulative bytes received.
    pub receive_bytes_total: u64,
    /// Cumulative bytes transmitted.
    pub transmit_bytes_total: u64,
    /// Assigned addresses, when reported.
    #[serde(default)]
    pub addresses: Option<Vec<String>>,
}

/// Resource usage of the booth process itself.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothProcessStats {
    /// Resident memory in bytes.
    #[serde(default)]
    pub resident_bytes: Option<u64>,
    /// Virtual memory in bytes.
    #[serde(default)]
    pub virtual_bytes: Option<u64>,
    /// Open file descriptors.
    #[serde(default)]
    pub open_fds: Option<u64>,
    /// Thread count.
    #[serde(default)]
    pub threads: Option<u64>,
    /// Process uptime in seconds.
    #[serde(default)]
    pub uptime_seconds: Option<f64>,
}

/// Active audio devices and sample rate.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothAudioStats {
    /// Input (capture) device name.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Output (playback) device name.
    #[serde(default)]
    pub output_device: Option<String>,
    /// Sample rate in Hz.
    #[serde(default)]
    pub sample_rate_hz: Option<u32>,
}

/// Tailscale connectivity summary.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothTailscaleStats {
    /// Whether Tailscale is connected.
    #[serde(default)]
    pub connected: Option<bool>,
    /// Number of known peers.
    #[serde(default)]
    pub peer_count: Option<u32>,
    /// Tailscale hostname.
    #[serde(default)]
    pub hostname: Option<String>,
    /// Active exit node, if any.
    #[serde(default)]
    pub exit_node: Option<String>,
}

/// The six Raspberry Pi throttling flags (`vcgencmd get_throttled`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothThrottlingFlags {
    /// Under-voltage currently detected.
    #[serde(default)]
    pub undervoltage: Option<bool>,
    /// ARM frequency currently capped.
    #[serde(default)]
    pub arm_freq_capped: Option<bool>,
    /// Currently throttled.
    #[serde(default)]
    pub throttled: Option<bool>,
    /// Soft temperature limit active.
    #[serde(default)]
    pub soft_temp_limit: Option<bool>,
    /// Under-voltage has occurred since boot.
    #[serde(default)]
    pub undervoltage_occurred: Option<bool>,
    /// Throttling has occurred since boot.
    #[serde(default)]
    pub throttled_occurred: Option<bool>,
}

/// A full live system snapshot.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothSystemSnapshot {
    /// CPU stats.
    #[serde(default)]
    pub cpu: Option<BoothCpuStats>,
    /// SoC/CPU temperature in Celsius.
    #[serde(default)]
    pub temperature_celsius: Option<f64>,
    /// Memory stats.
    #[serde(default)]
    pub memory: Option<BoothMemoryStats>,
    /// Per-mount disk stats.
    #[serde(default)]
    pub disks: Option<Vec<BoothDiskStats>>,
    /// Per-interface network stats.
    #[serde(default)]
    pub networks: Option<Vec<BoothNetworkStats>>,
    /// Host uptime in seconds.
    #[serde(default)]
    pub uptime_seconds: Option<f64>,
    /// Booth process stats.
    #[serde(default)]
    pub process: Option<BoothProcessStats>,
    /// Audio device stats.
    #[serde(default)]
    pub audio: Option<BoothAudioStats>,
    /// Tailscale stats.
    #[serde(default)]
    pub tailscale: Option<BoothTailscaleStats>,
    /// Pi throttling flags.
    #[serde(default)]
    pub throttling: Option<BoothThrottlingFlags>,
    /// How the booth is being driven.
    #[serde(default)]
    pub runtime_mode: Option<RuntimeMode>,
}

/// Server-side envelope around a snapshot (`GET /v1/system/current`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothSystemSnapshotEnvelope {
    /// Booth identifier.
    pub booth_id: String,
    /// The snapshot itself.
    pub snapshot: BoothSystemSnapshot,
    /// When the server received the snapshot.
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: time::OffsetDateTime,
    /// Running booth client version, when reported.
    #[serde(default)]
    pub version: Option<String>,
}

/// List of per-booth live system snapshots (`GET /v1/system/current` with no
/// `boothId`, which returns every cached booth).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoothSystemSnapshotList {
    /// One latest snapshot per known booth.
    pub items: Vec<BoothSystemSnapshotEnvelope>,
}
