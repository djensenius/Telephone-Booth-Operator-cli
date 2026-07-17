//! Wire-contract domain types mirroring the operator API.
//!
//! Each submodule corresponds to a slice of the operator API surface. All
//! structs derive [`serde::Serialize`] + [`serde::Deserialize`] with
//! `#[serde(rename_all = "camelCase")]` so they round-trip the JSON the API
//! produces and accepts. Timestamps are decoded as [`time::OffsetDateTime`]
//! from RFC 3339 strings; identifiers and opaque blobs stay as [`String`] /
//! [`serde_json::Value`] to stay forward-compatible.

pub mod admin;
pub mod ai;
pub mod booth;
pub mod common;
pub mod event;
pub mod message;
pub mod operator;
pub mod question;
pub mod session;
pub mod stats;
pub mod system;
pub mod token;
pub mod upload;
pub mod ws;

pub use admin::DataImportSummary;
pub use ai::{
    AiProvider, Moderation, ModerationRecommendation, Transcription, TranscriptionList,
    TranscriptionStatus,
};
pub use booth::{BoothState, BoothStatus, RuntimeMode, StatusHistory, StatusUpdate};
pub use common::AudioRef;
pub use event::{BoothEvent, BoothEventList, BoothEventRecord, BoothEventType};
pub use message::{
    Message, MessageDecision, MessageDecisionKind, MessageList, MessageStatus, TranslationSubmit,
};
pub use operator::OperatorMe;
pub use question::{Question, QuestionCreate, QuestionList, QuestionStatus};
pub use session::{CallOutcome, CallSession, CallSessionDetail, CallSessionList};
pub use stats::{
    MetricFilter, MetricFilterInput, StatsBoothBreakdown, StatsBusiest, StatsCallsPerDay,
    StatsHourlyBucket, StatsOverview, StatsTopQuestion, StatsWindow,
};
pub use system::{
    BoothAudioStats, BoothCpuStats, BoothDiskStats, BoothMemoryStats, BoothNetworkStats,
    BoothProcessStats, BoothSystemSnapshot, BoothSystemSnapshotEnvelope, BoothSystemSnapshotList,
    BoothTailscaleStats, BoothThrottlingFlags,
};
pub use token::{ApiToken, ApiTokenCreated, ApiTokenUsageBucket, CreateApiTokenRequest};
pub use upload::{UploadSasKind, UploadSasRequest, UploadSlot};
pub use ws::WsEnvelope;
