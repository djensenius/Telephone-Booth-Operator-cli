//! Booth debug-server client for the `tb-operator` console.
//!
//! Connects directly to a booth's on-device debug server: REST snapshots
//! (state, GPIO, audio, system, logs, config), the telemetry WebSocket, the
//! Prometheus `/metrics` endpoint, and event simulation. LAN connections use a
//! pinned self-signed certificate. The client is added in a later phase.
