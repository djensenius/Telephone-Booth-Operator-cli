//! Operator API client for the `tb-operator` console.
//!
//! Wraps the Authentik-secured operator REST API, covering the read endpoints
//! for status, messages, questions, events, and sessions, plus the
//! bearer-authenticated server-sent events stream (`/v1/events/stream`) for a
//! live event tail. Mutating actions cover message moderation (approve/reject,
//! translation submit, re-transcribe/re-moderate, delete), question
//! management (activate/deactivate/archive, plus create via the audio-upload
//! SAS flow), and API-token management (list, create with a one-time plaintext
//! secret, revoke, and per-token usage). The client is generic over an
//! [`HttpTransport`] (so request construction can be unit-tested without a
//! network) and a [`TokenProvider`] (so the bearer token can come from the
//! authenticated session or a fixed value).

mod client;
mod error;
mod sse;
mod token;
mod transport;

pub use client::{BoothEventStream, EventQuery, OperatorClient};
pub use error::{OperatorError, Result};
pub use sse::{SseEvent, SseParser};
pub use token::{StaticTokenProvider, TokenProvider};
pub use transport::{
    ByteStream, HttpResponse, HttpTransport, ReqwestTransport, SseTransport, WriteTransport,
};
