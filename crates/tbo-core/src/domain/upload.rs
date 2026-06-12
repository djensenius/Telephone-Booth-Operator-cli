//! Audio upload (SAS) flow used when creating questions.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// Which kind of audio a SAS slot is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UploadSasKind {
    /// A caller message recording.
    Message,
    /// A question prompt recording.
    QuestionAudio,
}

/// Body for `POST /v1/uploads/sas`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSasRequest {
    /// What the upload is for.
    pub kind: UploadSasKind,
    /// Lower-case hex SHA-256 of the bytes to upload.
    pub sha256: String,
    /// Size of the upload in bytes.
    pub size_bytes: u64,
    /// Always `audio/flac`.
    pub content_type: String,
}

/// A short-lived upload slot returned by the SAS endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadSlot {
    /// PUT target for the blob.
    pub upload_url: String,
    /// Server-assigned blob name.
    pub blob_name: String,
    /// When the slot expires.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Audio file id to reference once the upload completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_file_id: Option<String>,
}
