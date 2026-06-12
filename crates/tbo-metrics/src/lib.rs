//! Prometheus metrics handling for the `tb-operator` system-health dashboard.
//!
//! This crate is network-free: it turns the booth debug server's Prometheus
//! `/metrics` text exposition into chartable data. It has three layers:
//!
//! - [`parse`] — a lenient parser for the Prometheus text exposition format
//!   producing a [`MetricSet`] of [`Sample`]s.
//! - [`snapshot`] — extraction of the booth's known `booth_*` series into a
//!   typed [`BoothMetrics`] snapshot.
//! - [`series`] — [`RingBuffer`], [`RateMeter`], and [`MetricsHistory`] for the
//!   rolling, `btm`-style charts (including counter-to-rate conversion for the
//!   network byte counters).

pub mod parse;
pub mod series;
pub mod snapshot;

pub use parse::{MetricSet, MetricType, Sample};
pub use series::{MetricsHistory, RateMeter, RingBuffer};
pub use snapshot::{BoothMetrics, DiskUsage, NetworkCounters};
