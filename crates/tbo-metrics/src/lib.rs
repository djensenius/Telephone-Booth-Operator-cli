//! Prometheus metrics handling for the `tb-operator` system-health dashboard.
//!
//! Parses the booth's Prometheus text exposition format, maintains ring
//! buffers of recent samples for `btm`-style charts, and converts counter
//! series (e.g. network bytes) into per-second rates. Parsing and buffering are
//! added in a later phase.
