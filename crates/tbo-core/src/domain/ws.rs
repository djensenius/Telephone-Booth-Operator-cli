//! Discriminated union pushed over the `/v1/ws/status` socket.
//!
//! Native clients such as the TUI subscribe with bearer authentication; browser
//! clients may also use the server-side cookie flow. The shared envelope shape
//! lives in core so every client decodes the same payloads.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use super::booth::BoothStatus;
use super::message::Message;
use super::system::BoothSystemSnapshot;

/// One frame from the operator status socket, tagged by `kind`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WsEnvelope {
    /// A booth status update.
    Status {
        /// The new status.
        status: BoothStatus,
    },
    /// A live system snapshot.
    #[serde(rename_all = "camelCase")]
    System {
        /// Booth identifier.
        booth_id: String,
        /// The snapshot (boxed to keep the enum variants similarly sized).
        snapshot: Box<BoothSystemSnapshot>,
        /// When the server received it.
        #[serde(with = "time::serde::rfc3339")]
        received_at: OffsetDateTime,
        /// Booth client version, when reported.
        #[serde(default)]
        version: Option<String>,
    },
    /// A new or updated message.
    Message {
        /// The message (boxed to keep the enum variants similarly sized).
        message: Box<Message>,
    },
}
