//! Time-series buffering for the `btm`-style dashboard.
//!
//! The dashboard renders rolling charts from a stream of [`BoothMetrics`]
//! snapshots. This module provides the three primitives that turn discrete
//! scrapes into chartable series:
//!
//! - [`RingBuffer`] — a fixed-capacity FIFO of the most recent samples.
//! - [`RateMeter`] — converts a cumulative counter into a per-second rate,
//!   tolerating counter resets.
//! - [`MetricsHistory`] — ingests timestamped snapshots and maintains the
//!   derived series (CPU%, memory%, temperature, load, and network rates) the
//!   dashboard plots.

use std::collections::VecDeque;
use std::time::Instant;

use crate::snapshot::BoothMetrics;

/// A fixed-capacity FIFO buffer that retains only the most recent `capacity`
/// values, evicting the oldest when full.
#[derive(Debug, Clone)]
pub struct RingBuffer<T> {
    buf: VecDeque<T>,
    capacity: usize,
}

impl<T> RingBuffer<T> {
    /// Creates a buffer holding at most `capacity` values (clamped to a minimum
    /// of one).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            buf: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Appends `value`, evicting the oldest entry if the buffer is full.
    pub fn push(&mut self, value: T) {
        if self.buf.len() == self.capacity {
            self.buf.pop_front();
        }
        self.buf.push_back(value);
    }

    /// The number of buffered values.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// The maximum number of values the buffer retains.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Iterates the buffered values from oldest to newest.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.buf.iter()
    }

    /// The most recently pushed value, if any.
    pub fn last(&self) -> Option<&T> {
        self.buf.back()
    }
}

impl<T: Clone> RingBuffer<T> {
    /// Collects the buffered values into a `Vec`, oldest first.
    pub fn to_vec(&self) -> Vec<T> {
        self.buf.iter().cloned().collect()
    }
}

/// Converts a monotonically increasing counter into a per-second rate.
///
/// Each [`observe`](RateMeter::observe) call returns the average rate since the
/// previous observation. The first observation establishes a baseline and
/// returns `None`. A counter that decreases (a reset, e.g. a booth reboot)
/// yields a rate of `0.0` rather than a negative value.
#[derive(Debug, Clone, Default)]
pub struct RateMeter {
    last: Option<(Instant, f64)>,
}

impl RateMeter {
    /// Creates a meter with no prior observation.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records the counter `total` observed at `at` and returns the per-second
    /// rate since the previous observation, or `None` for the first call or a
    /// non-positive time delta.
    pub fn observe(&mut self, at: Instant, total: f64) -> Option<f64> {
        let rate = self.last.and_then(|(previous_at, previous_total)| {
            let seconds = at.saturating_duration_since(previous_at).as_secs_f64();
            if seconds <= 0.0 {
                return None;
            }
            let delta = (total - previous_total).max(0.0);
            Some(delta / seconds)
        });
        self.last = Some((at, total));
        rate
    }

    /// Discards the baseline so the next [`observe`](RateMeter::observe) starts
    /// fresh.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

/// Rolling history of the derived system-health series the dashboard plots.
///
/// Call [`record`](MetricsHistory::record) once per scrape. Scalar gauges are
/// buffered directly; the aggregate network counters are differenced into
/// per-second rates via internal [`RateMeter`]s. Each series is independently
/// buffered, so a snapshot missing one series does not disturb the others. The
/// full latest snapshot is retained for instantaneous read-outs (e.g. per-disk
/// usage).
#[derive(Debug, Clone)]
pub struct MetricsHistory {
    cpu_usage: RingBuffer<f64>,
    memory_ratio: RingBuffer<f64>,
    temperature_c: RingBuffer<f64>,
    load_1m: RingBuffer<f64>,
    net_receive_rate: RingBuffer<f64>,
    net_transmit_rate: RingBuffer<f64>,
    rx_meter: RateMeter,
    tx_meter: RateMeter,
    latest: Option<BoothMetrics>,
}

impl MetricsHistory {
    /// Creates a history whose every series buffers up to `capacity` samples.
    pub fn new(capacity: usize) -> Self {
        Self {
            cpu_usage: RingBuffer::new(capacity),
            memory_ratio: RingBuffer::new(capacity),
            temperature_c: RingBuffer::new(capacity),
            load_1m: RingBuffer::new(capacity),
            net_receive_rate: RingBuffer::new(capacity),
            net_transmit_rate: RingBuffer::new(capacity),
            rx_meter: RateMeter::new(),
            tx_meter: RateMeter::new(),
            latest: None,
        }
    }

    /// Ingests a snapshot scraped at `at`, updating every derived series for
    /// which the snapshot carries data.
    pub fn record(&mut self, at: Instant, metrics: BoothMetrics) {
        if let Some(cpu) = metrics.cpu_usage_ratio {
            self.cpu_usage.push(cpu);
        }
        if let Some(ratio) = metrics.memory_used_ratio() {
            self.memory_ratio.push(ratio);
        }
        if let Some(temp) = metrics.cpu_temperature_celsius {
            self.temperature_c.push(temp);
        }
        if let Some(load) = metrics.load_average_1m {
            self.load_1m.push(load);
        }
        if let Some(received) = metrics.total_receive_bytes()
            && let Some(rate) = self.rx_meter.observe(at, received)
        {
            self.net_receive_rate.push(rate);
        }
        if let Some(transmitted) = metrics.total_transmit_bytes()
            && let Some(rate) = self.tx_meter.observe(at, transmitted)
        {
            self.net_transmit_rate.push(rate);
        }
        self.latest = Some(metrics);
    }

    /// Buffered CPU usage ratios (`0.0`–`1.0`), oldest first.
    pub fn cpu_usage(&self) -> &RingBuffer<f64> {
        &self.cpu_usage
    }

    /// Buffered memory usage ratios (`0.0`–`1.0`), oldest first.
    pub fn memory_ratio(&self) -> &RingBuffer<f64> {
        &self.memory_ratio
    }

    /// Buffered CPU temperatures in degrees Celsius, oldest first.
    pub fn temperature_c(&self) -> &RingBuffer<f64> {
        &self.temperature_c
    }

    /// Buffered one-minute load averages, oldest first.
    pub fn load_1m(&self) -> &RingBuffer<f64> {
        &self.load_1m
    }

    /// Buffered aggregate network receive rates in bytes per second.
    pub fn net_receive_rate(&self) -> &RingBuffer<f64> {
        &self.net_receive_rate
    }

    /// Buffered aggregate network transmit rates in bytes per second.
    pub fn net_transmit_rate(&self) -> &RingBuffer<f64> {
        &self.net_transmit_rate
    }

    /// The most recently recorded snapshot, for instantaneous read-outs.
    pub fn latest(&self) -> Option<&BoothMetrics> {
        self.latest.as_ref()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

    use std::time::Duration;

    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest() {
        let mut buf = RingBuffer::new(3);
        for v in [1, 2, 3, 4, 5] {
            buf.push(v);
        }
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.capacity(), 3);
        assert_eq!(buf.to_vec(), vec![3, 4, 5]);
        assert_eq!(buf.last(), Some(&5));
    }

    #[test]
    fn ring_buffer_capacity_is_clamped() {
        let mut buf = RingBuffer::new(0);
        assert_eq!(buf.capacity(), 1);
        assert!(buf.is_empty());
        buf.push(7);
        buf.push(8);
        assert_eq!(buf.to_vec(), vec![8]);
    }

    #[test]
    fn rate_meter_first_observation_has_no_rate() {
        let mut meter = RateMeter::new();
        let t0 = Instant::now();
        assert_eq!(meter.observe(t0, 100.0), None);
    }

    #[test]
    fn rate_meter_computes_per_second_rate() {
        let mut meter = RateMeter::new();
        let t0 = Instant::now();
        assert_eq!(meter.observe(t0, 100.0), None);
        // 200 bytes over 2 seconds = 100 bytes/sec.
        let rate = meter.observe(t0 + Duration::from_secs(2), 300.0);
        assert_eq!(rate, Some(100.0));
    }

    #[test]
    fn rate_meter_handles_counter_reset() {
        let mut meter = RateMeter::new();
        let t0 = Instant::now();
        meter.observe(t0, 500.0);
        // Counter went backwards (reset) -> clamp to zero, not negative.
        let rate = meter.observe(t0 + Duration::from_secs(1), 10.0);
        assert_eq!(rate, Some(0.0));
    }

    #[test]
    fn rate_meter_rejects_zero_delta_time() {
        let mut meter = RateMeter::new();
        let t0 = Instant::now();
        meter.observe(t0, 100.0);
        assert_eq!(meter.observe(t0, 200.0), None);
    }

    #[test]
    fn rate_meter_reset_clears_baseline() {
        let mut meter = RateMeter::new();
        let t0 = Instant::now();
        meter.observe(t0, 100.0);
        meter.reset();
        assert_eq!(meter.observe(t0 + Duration::from_secs(1), 200.0), None);
    }

    fn snapshot(cpu: f64, rx: f64, tx: f64) -> BoothMetrics {
        BoothMetrics {
            cpu_usage_ratio: Some(cpu),
            memory_used_bytes: Some(500.0),
            memory_total_bytes: Some(1000.0),
            cpu_temperature_celsius: Some(40.0),
            load_average_1m: Some(0.5),
            networks: vec![crate::snapshot::NetworkCounters {
                iface: "eth0".to_owned(),
                receive_bytes_total: Some(rx),
                transmit_bytes_total: Some(tx),
            }],
            ..BoothMetrics::default()
        }
    }

    #[test]
    fn history_buffers_scalars_each_record() {
        let mut history = MetricsHistory::new(8);
        let t0 = Instant::now();
        history.record(t0, snapshot(0.2, 0.0, 0.0));
        history.record(t0 + Duration::from_secs(1), snapshot(0.4, 0.0, 0.0));

        assert_eq!(history.cpu_usage().to_vec(), vec![0.2, 0.4]);
        assert_eq!(history.memory_ratio().to_vec(), vec![0.5, 0.5]);
        assert_eq!(history.temperature_c().to_vec(), vec![40.0, 40.0]);
        assert_eq!(history.load_1m().to_vec(), vec![0.5, 0.5]);
    }

    #[test]
    fn history_derives_network_rates_after_first_sample() {
        let mut history = MetricsHistory::new(8);
        let t0 = Instant::now();
        history.record(t0, snapshot(0.2, 1000.0, 500.0));
        // No rate yet from a single observation.
        assert!(history.net_receive_rate().is_empty());

        history.record(t0 + Duration::from_secs(2), snapshot(0.3, 3000.0, 1500.0));
        // rx: 2000 bytes / 2s = 1000; tx: 1000 bytes / 2s = 500.
        assert_eq!(history.net_receive_rate().to_vec(), vec![1000.0]);
        assert_eq!(history.net_transmit_rate().to_vec(), vec![500.0]);
    }

    #[test]
    fn history_retains_latest_snapshot() {
        let mut history = MetricsHistory::new(8);
        let t0 = Instant::now();
        history.record(t0, snapshot(0.2, 0.0, 0.0));
        history.record(t0 + Duration::from_secs(1), snapshot(0.9, 0.0, 0.0));
        assert_eq!(history.latest().and_then(|m| m.cpu_usage_ratio), Some(0.9));
    }

    #[test]
    fn history_skips_absent_series() {
        let mut history = MetricsHistory::new(8);
        let t0 = Instant::now();
        // A snapshot with no CPU data must not push a CPU sample.
        history.record(t0, BoothMetrics::default());
        assert!(history.cpu_usage().is_empty());
        assert!(history.latest().is_some());
    }
}
