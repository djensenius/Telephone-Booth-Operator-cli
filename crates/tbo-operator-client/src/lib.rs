//! Operator API client for the `tb-operator` console.
//!
//! Wraps the Authentik-secured operator REST API and its bearer-authenticated
//! server-sent events stream (`/v1/events/stream`), covering status, messages,
//! questions, events, sessions, stats, and API tokens. Endpoints are added in
//! later phases.
