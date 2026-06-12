//! Operator API client for the `tb-operator` console.
//!
//! Wraps the Authentik-secured operator REST API, covering the read endpoints
//! for status, messages, questions, events, and sessions. The client is generic
//! over an [`HttpTransport`] (so request construction can be unit-tested without
//! a network) and a [`TokenProvider`] (so the bearer token can come from the
//! authenticated session or a fixed value).
//!
//! The bearer-authenticated server-sent events stream (`/v1/events/stream`),
//! the system snapshot, stats, and API-token endpoints are added in later
//! phases alongside the screens that consume them.

mod client;
mod error;
mod token;
mod transport;

pub use client::{EventQuery, OperatorClient};
pub use error::{OperatorError, Result};
pub use token::{StaticTokenProvider, TokenProvider};
pub use transport::{HttpResponse, HttpTransport, ReqwestTransport};
