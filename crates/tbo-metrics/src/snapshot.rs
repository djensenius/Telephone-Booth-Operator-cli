//! Extraction of the booth's known system-health series from a parsed
//! [`MetricSet`] into a strongly typed [`BoothMetrics`] snapshot.
//!
//! The booth exposes its system telemetry under the `booth_*` namespace (see
//! the `booth-metrics` crate in the Telephone-Booth monorepo). Every series
//! carries a global `booth_id` label; per-resource series add `window`,
//! `mountpoint`, or `iface` labels. This module pulls those specific series out
//! of the generic [`MetricSet`] so the dashboard can render them without
//! knowing the wire format.

use std::collections::BTreeMap;

use crate::parse::MetricSet;

/// Metric name for the CPU usage gauge (`0.0`–`1.0`).
const CPU_USAGE: &str = "booth_cpu_usage_ratio";
/// Metric name for the CPU temperature gauge (degrees Celsius).
const CPU_TEMP: &str = "booth_cpu_temperature_celsius";
/// Metric name for the load-average gauge (selected by the `window` label).
const LOAD_AVERAGE: &str = "booth_load_average";
/// Metric name for used physical memory in bytes.
const MEMORY_USED: &str = "booth_memory_used_bytes";
/// Metric name for total physical memory in bytes.
const MEMORY_TOTAL: &str = "booth_memory_total_bytes";
/// Metric name for used disk space in bytes (per `mountpoint`).
const DISK_USED: &str = "booth_disk_used_bytes";
/// Metric name for total disk space in bytes (per `mountpoint`).
const DISK_TOTAL: &str = "booth_disk_total_bytes";
/// Metric name for cumulative bytes received (per `iface`).
const NET_RECEIVE: &str = "booth_network_receive_bytes_total";
/// Metric name for cumulative bytes transmitted (per `iface`).
const NET_TRANSMIT: &str = "booth_network_transmit_bytes_total";
/// Metric name for the system uptime gauge (seconds).
const UPTIME: &str = "booth_uptime_seconds";

/// Per-mountpoint disk usage extracted from `booth_disk_*_bytes`.
#[derive(Debug, Clone, PartialEq)]
pub struct DiskUsage {
    /// The filesystem mount point (the `mountpoint` label).
    pub mountpoint: String,
    /// Bytes used, if the `booth_disk_used_bytes` series was present.
    pub used_bytes: Option<f64>,
    /// Total capacity in bytes, if `booth_disk_total_bytes` was present.
    pub total_bytes: Option<f64>,
}

impl DiskUsage {
    /// The fraction of the filesystem in use (`0.0`–`1.0`), if both the used
    /// and a non-zero total are known.
    pub fn used_ratio(&self) -> Option<f64> {
        match (self.used_bytes, self.total_bytes) {
            (Some(used), Some(total)) if total > 0.0 => Some(used / total),
            _ => None,
        }
    }
}

/// Per-interface cumulative network counters from `booth_network_*_bytes_total`.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkCounters {
    /// The network interface name (the `iface` label).
    pub iface: String,
    /// Cumulative bytes received, if the receive counter was present.
    pub receive_bytes_total: Option<f64>,
    /// Cumulative bytes transmitted, if the transmit counter was present.
    pub transmit_bytes_total: Option<f64>,
}

/// A point-in-time view of the booth's system metrics from one `/metrics`
/// scrape.
///
/// All fields are optional because a scrape may omit a series (for example, a
/// booth without temperature sensing has no `booth_cpu_temperature_celsius`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BoothMetrics {
    /// CPU usage as a fraction (`0.0`–`1.0`).
    pub cpu_usage_ratio: Option<f64>,
    /// CPU temperature in degrees Celsius.
    pub cpu_temperature_celsius: Option<f64>,
    /// One-minute load average.
    pub load_average_1m: Option<f64>,
    /// Five-minute load average.
    pub load_average_5m: Option<f64>,
    /// Fifteen-minute load average.
    pub load_average_15m: Option<f64>,
    /// Used physical memory in bytes.
    pub memory_used_bytes: Option<f64>,
    /// Total physical memory in bytes.
    pub memory_total_bytes: Option<f64>,
    /// System uptime in seconds.
    pub uptime_seconds: Option<f64>,
    /// Per-mountpoint disk usage, ordered by mount point.
    pub disks: Vec<DiskUsage>,
    /// Per-interface network counters, ordered by interface name.
    pub networks: Vec<NetworkCounters>,
}

impl BoothMetrics {
    /// Extracts the booth's known series from a parsed [`MetricSet`].
    pub fn from_metric_set(set: &MetricSet) -> Self {
        Self {
            cpu_usage_ratio: set.value(CPU_USAGE),
            cpu_temperature_celsius: set.value(CPU_TEMP),
            load_average_1m: set.value_where(LOAD_AVERAGE, &[("window", "1m")]),
            load_average_5m: set.value_where(LOAD_AVERAGE, &[("window", "5m")]),
            load_average_15m: set.value_where(LOAD_AVERAGE, &[("window", "15m")]),
            memory_used_bytes: set.value(MEMORY_USED),
            memory_total_bytes: set.value(MEMORY_TOTAL),
            uptime_seconds: set.value(UPTIME),
            disks: extract_disks(set),
            networks: extract_networks(set),
        }
    }

    /// Parses a Prometheus exposition and extracts the booth's known series.
    pub fn parse(text: &str) -> Self {
        Self::from_metric_set(&MetricSet::parse(text))
    }

    /// The fraction of physical memory in use (`0.0`–`1.0`), if both the used
    /// and a non-zero total are known.
    pub fn memory_used_ratio(&self) -> Option<f64> {
        match (self.memory_used_bytes, self.memory_total_bytes) {
            (Some(used), Some(total)) if total > 0.0 => Some(used / total),
            _ => None,
        }
    }

    /// Total cumulative bytes received across all interfaces.
    ///
    /// Returns `None` only when no interface reported a receive counter.
    pub fn total_receive_bytes(&self) -> Option<f64> {
        sum_counters(self.networks.iter().map(|n| n.receive_bytes_total))
    }

    /// Total cumulative bytes transmitted across all interfaces.
    ///
    /// Returns `None` only when no interface reported a transmit counter.
    pub fn total_transmit_bytes(&self) -> Option<f64> {
        sum_counters(self.networks.iter().map(|n| n.transmit_bytes_total))
    }
}

/// Sums an iterator of optional counter values, yielding `None` if every value
/// is absent.
fn sum_counters(values: impl Iterator<Item = Option<f64>>) -> Option<f64> {
    let mut total = None;
    for value in values.flatten() {
        total = Some(total.unwrap_or(0.0) + value);
    }
    total
}

/// Collects per-mountpoint disk usage, ordered by mount point.
fn extract_disks(set: &MetricSet) -> Vec<DiskUsage> {
    let mut disks: BTreeMap<String, DiskUsage> = BTreeMap::new();
    for sample in set.series(DISK_USED) {
        if let Some(mount) = sample.label("mountpoint") {
            disks
                .entry(mount.to_owned())
                .or_insert_with(|| new_disk(mount))
                .used_bytes = Some(sample.value);
        }
    }
    for sample in set.series(DISK_TOTAL) {
        if let Some(mount) = sample.label("mountpoint") {
            disks
                .entry(mount.to_owned())
                .or_insert_with(|| new_disk(mount))
                .total_bytes = Some(sample.value);
        }
    }
    disks.into_values().collect()
}

/// Collects per-interface network counters, ordered by interface name.
fn extract_networks(set: &MetricSet) -> Vec<NetworkCounters> {
    let mut nets: BTreeMap<String, NetworkCounters> = BTreeMap::new();
    for sample in set.series(NET_RECEIVE) {
        if let Some(iface) = sample.label("iface") {
            nets.entry(iface.to_owned())
                .or_insert_with(|| new_net(iface))
                .receive_bytes_total = Some(sample.value);
        }
    }
    for sample in set.series(NET_TRANSMIT) {
        if let Some(iface) = sample.label("iface") {
            nets.entry(iface.to_owned())
                .or_insert_with(|| new_net(iface))
                .transmit_bytes_total = Some(sample.value);
        }
    }
    nets.into_values().collect()
}

/// Builds an empty [`DiskUsage`] for `mount`.
fn new_disk(mount: &str) -> DiskUsage {
    DiskUsage {
        mountpoint: mount.to_owned(),
        used_bytes: None,
        total_bytes: None,
    }
}

/// Builds an empty [`NetworkCounters`] for `iface`.
fn new_net(iface: &str) -> NetworkCounters {
    NetworkCounters {
        iface: iface.to_owned(),
        receive_bytes_total: None,
        transmit_bytes_total: None,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use super::*;

    const EXPOSITION: &str = r#"
# TYPE booth_cpu_usage_ratio gauge
booth_cpu_usage_ratio{booth_id="b"} 0.5
# TYPE booth_cpu_temperature_celsius gauge
booth_cpu_temperature_celsius{booth_id="b"} 42
# TYPE booth_load_average gauge
booth_load_average{booth_id="b",window="1m"} 0.1
booth_load_average{booth_id="b",window="5m"} 0.2
booth_load_average{booth_id="b",window="15m"} 0.3
# TYPE booth_memory_used_bytes gauge
booth_memory_used_bytes{booth_id="b"} 250
booth_memory_total_bytes{booth_id="b"} 1000
# TYPE booth_disk_used_bytes gauge
booth_disk_used_bytes{booth_id="b",mountpoint="/"} 30
booth_disk_total_bytes{booth_id="b",mountpoint="/"} 100
booth_disk_used_bytes{booth_id="b",mountpoint="/boot"} 5
booth_disk_total_bytes{booth_id="b",mountpoint="/boot"} 50
# TYPE booth_network_receive_bytes_total counter
booth_network_receive_bytes_total{booth_id="b",iface="eth0"} 1000
booth_network_transmit_bytes_total{booth_id="b",iface="eth0"} 500
booth_network_receive_bytes_total{booth_id="b",iface="wlan0"} 200
# TYPE booth_uptime_seconds gauge
booth_uptime_seconds{booth_id="b"} 3600
"#;

    #[test]
    fn extracts_scalar_gauges() {
        let metrics = BoothMetrics::parse(EXPOSITION);
        assert_eq!(metrics.cpu_usage_ratio, Some(0.5));
        assert_eq!(metrics.cpu_temperature_celsius, Some(42.0));
        assert_eq!(metrics.uptime_seconds, Some(3600.0));
        assert_eq!(metrics.memory_used_bytes, Some(250.0));
        assert_eq!(metrics.memory_total_bytes, Some(1000.0));
    }

    #[test]
    fn extracts_load_average_windows() {
        let metrics = BoothMetrics::parse(EXPOSITION);
        assert_eq!(metrics.load_average_1m, Some(0.1));
        assert_eq!(metrics.load_average_5m, Some(0.2));
        assert_eq!(metrics.load_average_15m, Some(0.3));
    }

    #[test]
    fn pairs_disks_by_mountpoint_in_order() {
        let metrics = BoothMetrics::parse(EXPOSITION);
        assert_eq!(metrics.disks.len(), 2);
        // BTreeMap ordering: "/" sorts before "/boot".
        let root = &metrics.disks[0];
        assert_eq!(root.mountpoint, "/");
        assert_eq!(root.used_bytes, Some(30.0));
        assert_eq!(root.total_bytes, Some(100.0));
        assert_eq!(root.used_ratio(), Some(0.3));

        let boot = &metrics.disks[1];
        assert_eq!(boot.mountpoint, "/boot");
        assert_eq!(boot.used_ratio(), Some(0.1));
    }

    #[test]
    fn pairs_networks_by_iface() {
        let metrics = BoothMetrics::parse(EXPOSITION);
        assert_eq!(metrics.networks.len(), 2);
        let eth0 = &metrics.networks[0];
        assert_eq!(eth0.iface, "eth0");
        assert_eq!(eth0.receive_bytes_total, Some(1000.0));
        assert_eq!(eth0.transmit_bytes_total, Some(500.0));

        let wlan0 = &metrics.networks[1];
        assert_eq!(wlan0.iface, "wlan0");
        assert_eq!(wlan0.receive_bytes_total, Some(200.0));
        // wlan0 has no transmit series in the exposition.
        assert_eq!(wlan0.transmit_bytes_total, None);
    }

    #[test]
    fn aggregates_and_ratios() {
        let metrics = BoothMetrics::parse(EXPOSITION);
        assert_eq!(metrics.memory_used_ratio(), Some(0.25));
        assert_eq!(metrics.total_receive_bytes(), Some(1200.0));
        assert_eq!(metrics.total_transmit_bytes(), Some(500.0));
    }

    #[test]
    fn missing_series_yield_none() {
        let metrics = BoothMetrics::parse("");
        assert_eq!(metrics.cpu_usage_ratio, None);
        assert_eq!(metrics.memory_used_ratio(), None);
        assert_eq!(metrics.total_receive_bytes(), None);
        assert!(metrics.disks.is_empty());
        assert!(metrics.networks.is_empty());
    }

    #[test]
    fn zero_total_avoids_divide_by_zero() {
        let metrics =
            BoothMetrics::parse("booth_memory_used_bytes 10\nbooth_memory_total_bytes 0\n");
        assert_eq!(metrics.memory_used_ratio(), None);
    }
}
