//! The operator API client: typed read endpoints over an [`HttpTransport`]
//! and mutating actions over a [`WriteTransport`].

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::VecDeque;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use futures::stream::{BoxStream, Stream, StreamExt};

use tbo_core::domain::{
    ApiToken, ApiTokenCreated, ApiTokenUsageBucket, BoothEventList, BoothEventRecord, BoothStatus,
    BoothSystemSnapshotList, CallSessionDetail, CallSessionList, CreateApiTokenRequest, Message,
    MessageDecision, MessageDecisionKind, MessageList, MessageStatus, Moderation, OperatorMe,
    Question, QuestionCreate, QuestionList, QuestionStatus, StatsOverview, StatsWindow,
    StatusHistory, Transcription, TranscriptionList, TranslationSubmit, UploadSasKind,
    UploadSasRequest, UploadSlot,
};

use crate::error::{OperatorError, Result};
use crate::sse::SseParser;
use crate::token::{StaticTokenProvider, TokenProvider};
use crate::transport::{
    HttpResponse, HttpTransport, ReqwestTransport, SseTransport, WriteTransport,
};

/// Filter and pagination options for [`OperatorClient::events`] and the live
/// event stream.
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    /// Restrict to a single booth.
    pub booth_id: Option<String>,
    /// Restrict to a single call session.
    pub session_id: Option<String>,
    /// Only events at or after this instant.
    pub since: Option<OffsetDateTime>,
    /// Only events at or before this instant.
    pub until: Option<OffsetDateTime>,
    /// Restrict to these event type discriminators (repeated `type` params).
    pub types: Vec<String>,
    /// Opaque pagination cursor from a previous page.
    pub cursor: Option<String>,
    /// Maximum number of events to return (server caps at 500).
    pub limit: Option<u32>,
}

/// A client for the Authentik-secured operator REST API.
///
/// Generic over the HTTP [`HttpTransport`] (so calls can be tested without a
/// network) and a [`TokenProvider`] (so the bearer token can be sourced from
/// the authenticated session or a fixed value). Construct the production client
/// with [`OperatorClient::new`].
#[derive(Debug, Clone)]
pub struct OperatorClient<
    T: HttpTransport = ReqwestTransport,
    A: TokenProvider = StaticTokenProvider,
> {
    transport: T,
    auth: A,
}

impl<A: TokenProvider> OperatorClient<ReqwestTransport, A> {
    /// Build a client with a default rustls transport rooted at `base_url`
    /// (e.g. `https://api.telephonebooth.io`).
    ///
    /// # Errors
    /// Returns an error when the underlying HTTP client cannot be built.
    pub fn new(base_url: impl Into<String>, auth: A) -> Result<Self> {
        Ok(Self::with_transport(ReqwestTransport::new(base_url)?, auth))
    }
}

impl<T: HttpTransport, A: TokenProvider> OperatorClient<T, A> {
    /// Build a client over an explicit transport (used in tests).
    pub fn with_transport(transport: T, auth: A) -> Self {
        Self { transport, auth }
    }

    /// Current booth status (`GET /v1/status`). Public endpoint; the bearer is
    /// still sent when available.
    pub async fn status(&self) -> Result<BoothStatus> {
        self.get_json("/v1/status", &[], false).await
    }

    /// Recent booth status snapshots for charts (`GET /v1/status/history`).
    pub async fn status_history(
        &self,
        since: Option<OffsetDateTime>,
        limit: Option<u32>,
    ) -> Result<StatusHistory> {
        let mut query = Vec::new();
        if let Some(since) = since {
            query.push(("since", format_timestamp(since)?));
        }
        push_limit(&mut query, limit);
        self.get_json("/v1/status/history", &query, true).await
    }

    /// List messages, optionally filtered (`GET /v1/messages`).
    pub async fn messages(
        &self,
        status: Option<MessageStatus>,
        since: Option<OffsetDateTime>,
        limit: Option<u32>,
    ) -> Result<MessageList> {
        let mut query = Vec::new();
        if let Some(status) = status {
            query.push(("status", status.as_query().to_owned()));
        }
        if let Some(since) = since {
            query.push(("since", format_timestamp(since)?));
        }
        push_limit(&mut query, limit);
        self.get_json("/v1/messages", &query, true).await
    }

    /// Fetch a single message (`GET /v1/messages/{id}`).
    pub async fn message(&self, id: &str) -> Result<Message> {
        self.get_json(&format!("/v1/messages/{id}"), &[], true)
            .await
    }

    /// List a message's transcription attempts
    /// (`GET /v1/messages/{id}/transcriptions`).
    pub async fn message_transcriptions(&self, id: &str) -> Result<TranscriptionList> {
        self.get_json(&format!("/v1/messages/{id}/transcriptions"), &[], true)
            .await
    }

    /// List questions for management (`GET /v1/questions`).
    pub async fn questions(
        &self,
        status: Option<QuestionStatus>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<QuestionList> {
        let mut query = Vec::new();
        if let Some(status) = status {
            query.push(("status", status.as_query().to_owned()));
        }
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor.to_owned()));
        }
        push_limit(&mut query, limit);
        self.get_json("/v1/questions", &query, true).await
    }

    /// List booth events with filtering and pagination (`GET /v1/events`).
    pub async fn events(&self, filter: &EventQuery) -> Result<BoothEventList> {
        let mut query = Vec::new();
        if let Some(booth_id) = &filter.booth_id {
            query.push(("boothId", booth_id.clone()));
        }
        if let Some(session_id) = &filter.session_id {
            query.push(("sessionId", session_id.clone()));
        }
        if let Some(since) = filter.since {
            query.push(("since", format_timestamp(since)?));
        }
        if let Some(until) = filter.until {
            query.push(("until", format_timestamp(until)?));
        }
        for event_type in &filter.types {
            query.push(("type", event_type.clone()));
        }
        if let Some(cursor) = &filter.cursor {
            query.push(("cursor", cursor.clone()));
        }
        push_limit(&mut query, filter.limit);
        self.get_json("/v1/events", &query, true).await
    }

    /// List call sessions (`GET /v1/sessions`).
    pub async fn sessions(
        &self,
        booth_id: Option<&str>,
        cursor: Option<&str>,
        limit: Option<u32>,
    ) -> Result<CallSessionList> {
        let mut query = Vec::new();
        if let Some(booth_id) = booth_id {
            query.push(("boothId", booth_id.to_owned()));
        }
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor.to_owned()));
        }
        push_limit(&mut query, limit);
        self.get_json("/v1/sessions", &query, true).await
    }

    /// Fetch a single call session with its event timeline
    /// (`GET /v1/sessions/{id}`).
    pub async fn session(&self, id: &str) -> Result<CallSessionDetail> {
        self.get_json(&format!("/v1/sessions/{id}"), &[], true)
            .await
    }

    /// The signed-in operator's profile (`GET /v1/auth/me`).
    pub async fn operator_me(&self) -> Result<OperatorMe> {
        self.get_json("/v1/auth/me", &[], true).await
    }

    /// Usage statistics overview (`GET /v1/stats/overview`).
    ///
    /// `window` selects the aggregation range; when omitted the server defaults
    /// to the last 7 days.
    pub async fn stats_overview(&self, window: Option<StatsWindow>) -> Result<StatsOverview> {
        let mut query = Vec::new();
        if let Some(window) = window {
            query.push(("window", window.as_query().to_owned()));
        }
        self.get_json("/v1/stats/overview", &query, true).await
    }

    /// Latest live system snapshot for every known booth
    /// (`GET /v1/system/current`).
    ///
    /// Called without a `boothId`, the server returns the stable
    /// `{ items: [...] }` shape with one snapshot per booth.
    pub async fn system_current(&self) -> Result<BoothSystemSnapshotList> {
        self.get_json("/v1/system/current", &[], true).await
    }

    /// List the signed-in operator's API tokens (`GET /v1/api-tokens`).
    ///
    /// The endpoint returns a bare array (newest first); the secret is never
    /// included, only its `last4`.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error.
    pub async fn api_tokens(&self) -> Result<Vec<ApiToken>> {
        self.get_json("/v1/api-tokens", &[], true).await
    }

    /// Daily usage buckets for a single API token
    /// (`GET /v1/api-tokens/{id}/usage`).
    ///
    /// `days` bounds the look-back window (server default 30, max 365). The
    /// response is a bare array of buckets, empty when the token is unused
    /// within the window.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out,
    /// [`OperatorError::NotFound`] for an unknown id, or an HTTP/transport
    /// error.
    pub async fn api_token_usage(
        &self,
        id: &str,
        days: Option<u32>,
    ) -> Result<Vec<ApiTokenUsageBucket>> {
        let mut query = Vec::new();
        if let Some(days) = days {
            query.push(("days", days.to_string()));
        }
        self.get_json(&format!("/v1/api-tokens/{id}/usage"), &query, true)
            .await
    }

    /// Resolve the bearer token, issue the request, and decode the response.
    async fn get_json<R: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        require_auth: bool,
    ) -> Result<R> {
        let bearer = self.auth.access_token().await?;
        if require_auth && bearer.is_none() {
            return Err(OperatorError::Unauthenticated);
        }
        let response = self.transport.get(path, query, bearer.as_deref()).await?;
        if response.is_success() {
            serde_json::from_str(&response.body)
                .map_err(|err| OperatorError::Decode(err.to_string()))
        } else {
            Err(status_error(response))
        }
    }
}

/// Map a non-success [`HttpResponse`] to the matching [`OperatorError`].
fn status_error(response: HttpResponse) -> OperatorError {
    match response.status {
        401 | 403 => OperatorError::Unauthorized(response.status),
        404 => OperatorError::NotFound,
        status => OperatorError::Http {
            status,
            body: response.body,
        },
    }
}

/// The SSE `event:` name carrying a JSON [`BoothEventRecord`] payload.
const BOOTH_EVENT: &str = "booth-event";

impl<T: SseTransport, A: TokenProvider> OperatorClient<T, A> {
    /// Open the live event tail (`GET /v1/events/stream`).
    ///
    /// Returns a stream that yields each `booth-event` frame decoded into a
    /// [`BoothEventRecord`]; the `ready` handshake and `ping` heartbeats are
    /// transparently skipped. The stream ends when the connection closes.
    /// `boothId`, `sessionId`, and `type` filters from `filter` are forwarded;
    /// pagination fields (`since`/`until`/`cursor`/`limit`) do not apply.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or a
    /// transport/HTTP error if the stream cannot be opened.
    pub async fn events_stream(&self, filter: &EventQuery) -> Result<BoothEventStream> {
        let bearer = self.auth.access_token().await?;
        if bearer.is_none() {
            return Err(OperatorError::Unauthenticated);
        }
        let mut query = Vec::new();
        if let Some(booth_id) = &filter.booth_id {
            query.push(("boothId", booth_id.clone()));
        }
        if let Some(session_id) = &filter.session_id {
            query.push(("sessionId", session_id.clone()));
        }
        for event_type in &filter.types {
            query.push(("type", event_type.clone()));
        }
        let bytes = self
            .transport
            .get_sse("/v1/events/stream", &query, bearer.as_deref())
            .await?;
        Ok(decode_event_records(bytes).boxed())
    }
}

impl<T: WriteTransport, A: TokenProvider> OperatorClient<T, A> {
    /// Record an operator moderation decision
    /// (`POST /v1/messages/{id}/decision`).
    ///
    /// Approves or rejects the message, optionally attaching `notes`. The
    /// updated [`Message`] is returned.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error (the server replies `409` when the message is still
    /// uploading and therefore not yet decidable).
    pub async fn decide_message(
        &self,
        id: &str,
        decision: MessageDecisionKind,
        notes: Option<String>,
    ) -> Result<Message> {
        let body = MessageDecision { decision, notes };
        self.post_json(&format!("/v1/messages/{id}/decision"), &body)
            .await
    }

    /// Attach an operator-supplied translation to a message's latest succeeded
    /// transcription (`POST /v1/messages/{id}/translation`).
    ///
    /// Returns the updated [`Transcription`].
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error (the server replies `409` when the message has no
    /// succeeded transcription to translate).
    pub async fn submit_translation(
        &self,
        id: &str,
        translated_text: String,
        translated_language: Option<String>,
    ) -> Result<Transcription> {
        let body = TranslationSubmit {
            translated_text,
            translated_language,
        };
        self.post_json(&format!("/v1/messages/{id}/translation"), &body)
            .await
    }

    /// Queue a fresh transcription attempt for a message
    /// (`POST /v1/messages/{id}/transcribe`).
    ///
    /// Returns the newly created [`Transcription`] (server status `202`).
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error (the server replies `409` when an attempt is
    /// already pending).
    pub async fn retranscribe_message(&self, id: &str) -> Result<Transcription> {
        self.post_empty(&format!("/v1/messages/{id}/transcribe"))
            .await
    }

    /// Queue a fresh moderation pass for a message
    /// (`POST /v1/messages/{id}/moderate`).
    ///
    /// Returns the newly created [`Moderation`] (server status `202`).
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error (the server replies `409` when there is no
    /// succeeded transcription to moderate).
    pub async fn remoderate_message(&self, id: &str) -> Result<Moderation> {
        self.post_empty(&format!("/v1/messages/{id}/moderate"))
            .await
    }

    /// Permanently delete a message (`DELETE /v1/messages/{id}`).
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error.
    pub async fn delete_message(&self, id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/messages/{id}")).await
    }

    /// Create a new API token (`POST /v1/api-tokens`).
    ///
    /// Returns the created token including its one-time plaintext secret
    /// (server status `201`); the secret is never retrievable again, so it must
    /// be surfaced to the operator immediately. `expires_in_days` bounds the
    /// lifetime (max 3650); the token never expires when omitted.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error.
    pub async fn create_api_token(
        &self,
        name: String,
        expires_in_days: Option<u32>,
    ) -> Result<ApiTokenCreated> {
        let body = CreateApiTokenRequest {
            name,
            expires_in_days,
        };
        self.post_json("/v1/api-tokens", &body).await
    }

    /// Revoke an API token (`DELETE /v1/api-tokens/{id}`).
    ///
    /// Idempotent server-side: revoking an already-revoked or unknown token
    /// still returns `204`.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error.
    pub async fn revoke_api_token(&self, id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/api-tokens/{id}"))
            .await
    }

    /// Publish a question so callers can be offered it, clearing any prior
    /// retirement (`POST /v1/questions/{id}/activate`).
    ///
    /// Returns the updated [`Question`].
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out,
    /// [`OperatorError::NotFound`] for an unknown id, or an HTTP/transport
    /// error.
    pub async fn activate_question(&self, id: &str) -> Result<Question> {
        self.post_empty(&format!("/v1/questions/{id}/activate"))
            .await
    }

    /// Return a question to `draft` so it is no longer offered to callers
    /// (`POST /v1/questions/{id}/deactivate`).
    ///
    /// Returns the updated [`Question`].
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out,
    /// [`OperatorError::NotFound`] for an unknown id, or an HTTP/transport
    /// error.
    pub async fn deactivate_question(&self, id: &str) -> Result<Question> {
        self.post_empty(&format!("/v1/questions/{id}/deactivate"))
            .await
    }

    /// Archive (soft-delete) a question (`DELETE /v1/questions/{id}`).
    ///
    /// The booth stops offering the prompt, but existing messages are kept.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out,
    /// [`OperatorError::NotFound`] for an unknown or already-archived id, or an
    /// HTTP/transport error.
    pub async fn archive_question(&self, id: &str) -> Result<()> {
        self.delete_no_content(&format!("/v1/questions/{id}")).await
    }

    /// Create a question referencing a previously uploaded audio file
    /// (`POST /v1/questions`).
    ///
    /// New questions default to `draft` server-side unless `status` is set.
    /// Returns the created [`Question`] (server status `201`).
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out,
    /// [`OperatorError::NotFound`] when the audio file id is unknown, or an
    /// HTTP/transport error (the server replies `409` on a conflict).
    pub async fn create_question(
        &self,
        prompt: String,
        audio_file_id: String,
        status: Option<QuestionStatus>,
    ) -> Result<Question> {
        let body = QuestionCreate {
            prompt,
            audio_file_id,
            status,
        };
        self.post_json("/v1/questions", &body).await
    }

    /// Reserve a short-lived Azure blob SAS upload slot
    /// (`POST /v1/uploads/sas`).
    ///
    /// For [`UploadSasKind::QuestionAudio`] the returned [`UploadSlot`] carries
    /// the `audio_file_id` to reference when creating the question.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out, or an
    /// HTTP/transport error.
    pub async fn request_upload_slot(&self, request: &UploadSasRequest) -> Result<UploadSlot> {
        self.post_json("/v1/uploads/sas", request).await
    }

    /// Upload FLAC `audio` bytes to a question-audio blob slot, then create the
    /// question that references them.
    ///
    /// Performs the full three-step flow the web and mobile clients use:
    /// reserve a SAS slot for the bytes' SHA-256, `PUT` the bytes to the slot's
    /// URL, then `POST /v1/questions` with the slot's `audio_file_id`. New
    /// questions default to `draft` server-side unless `status` is set.
    ///
    /// # Errors
    /// Returns [`OperatorError::Unauthenticated`] when signed out,
    /// [`OperatorError::InvalidRequest`] when the SAS slot lacks an audio file
    /// id, or an HTTP/transport error from any step.
    pub async fn create_question_with_audio(
        &self,
        prompt: String,
        audio: Vec<u8>,
        status: Option<QuestionStatus>,
    ) -> Result<Question> {
        let sha256 = sha256_hex(&audio);
        let size_bytes = audio.len() as u64;
        let slot = self
            .request_upload_slot(&UploadSasRequest {
                kind: UploadSasKind::QuestionAudio,
                sha256,
                size_bytes,
                content_type: FLAC_CONTENT_TYPE.to_owned(),
            })
            .await?;
        let audio_file_id = slot.audio_file_id.ok_or_else(|| {
            OperatorError::InvalidRequest("upload slot did not include an audio file id".to_owned())
        })?;
        self.upload_audio_blob(&slot.upload_url, audio).await?;
        self.create_question(prompt, audio_file_id, status).await
    }

    /// `PUT` raw FLAC bytes to a blob SAS `url` (no bearer; the URL's SAS token
    /// is the credential).
    ///
    /// # Errors
    /// Returns an HTTP/transport error when the upload is rejected.
    async fn upload_audio_blob(&self, url: &str, audio: Vec<u8>) -> Result<()> {
        let response = self
            .transport
            .put_bytes(url, FLAC_CONTENT_TYPE, audio)
            .await?;
        if response.is_success() {
            Ok(())
        } else {
            Err(status_error(response))
        }
    }

    /// Resolve the bearer token, failing fast when signed out.
    async fn require_bearer(&self) -> Result<String> {
        self.auth
            .access_token()
            .await?
            .ok_or(OperatorError::Unauthenticated)
    }

    /// `POST` a JSON body and decode the JSON response.
    async fn post_json<B: Serialize + Sync, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R> {
        let json = serde_json::to_string(body)
            .map_err(|err| OperatorError::InvalidRequest(err.to_string()))?;
        let bearer = self.require_bearer().await?;
        let response = self
            .transport
            .post(path, &[], Some(&bearer), Some(&json))
            .await?;
        decode_success(response)
    }

    /// `POST` with no request body and decode the JSON response.
    async fn post_empty<R: DeserializeOwned>(&self, path: &str) -> Result<R> {
        let bearer = self.require_bearer().await?;
        let response = self.transport.post(path, &[], Some(&bearer), None).await?;
        decode_success(response)
    }

    /// `DELETE` an endpoint that returns no content on success.
    async fn delete_no_content(&self, path: &str) -> Result<()> {
        let bearer = self.require_bearer().await?;
        let response = self.transport.delete(path, &[], Some(&bearer)).await?;
        if response.is_success() {
            Ok(())
        } else {
            Err(status_error(response))
        }
    }
}

/// Decode a JSON success body, or map a non-success status to an error.
fn decode_success<R: DeserializeOwned>(response: HttpResponse) -> Result<R> {
    if response.is_success() {
        serde_json::from_str(&response.body).map_err(|err| OperatorError::Decode(err.to_string()))
    } else {
        Err(status_error(response))
    }
}

/// The fixed content type for booth audio uploads.
const FLAC_CONTENT_TYPE: &str = "audio/flac";

/// Lower-case hex SHA-256 of `bytes`, as the upload SAS endpoint expects.
fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// A boxed stream of live [`BoothEventRecord`]s from the operator event tail.
pub type BoothEventStream = BoxStream<'static, Result<BoothEventRecord>>;

/// Adapt a raw SSE byte stream into a stream of [`BoothEventRecord`]s, decoding
/// `booth-event` frames and skipping `ready`/`ping` and other event types.
fn decode_event_records<S>(bytes: S) -> impl Stream<Item = Result<BoothEventRecord>> + Send
where
    S: Stream<Item = Result<Vec<u8>>> + Send + Unpin + 'static,
{
    let state = (bytes, SseParser::new(), VecDeque::new());
    futures::stream::unfold(state, |(mut bytes, mut parser, mut pending)| async move {
        loop {
            if let Some(item) = pending.pop_front() {
                return Some((item, (bytes, parser, pending)));
            }
            match bytes.next().await {
                Some(Ok(chunk)) => {
                    for event in parser.push(&chunk) {
                        if event.event.as_deref() == Some(BOOTH_EVENT) {
                            pending.push_back(
                                serde_json::from_str::<BoothEventRecord>(&event.data)
                                    .map_err(|err| OperatorError::Decode(err.to_string())),
                            );
                        }
                    }
                }
                Some(Err(err)) => return Some((Err(err), (bytes, parser, pending))),
                None => return None,
            }
        }
    })
}

/// Append a `limit` query param when present.
fn push_limit(query: &mut Vec<(&'static str, String)>, limit: Option<u32>) {
    if let Some(limit) = limit {
        query.push(("limit", limit.to_string()));
    }
}

/// Format an [`OffsetDateTime`] as the RFC 3339 string the API expects.
fn format_timestamp(timestamp: OffsetDateTime) -> Result<String> {
    timestamp
        .format(&Rfc3339)
        .map_err(|err| OperatorError::InvalidRequest(format!("invalid timestamp: {err}")))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::transport::HttpResponse;

    /// A recorded request, captured for assertions.
    #[derive(Debug, Clone)]
    struct RecordedCall {
        method: &'static str,
        path: String,
        query: Vec<(String, String)>,
        bearer: Option<String>,
        body: Option<String>,
    }

    #[derive(Default)]
    struct FakeState {
        responses: VecDeque<HttpResponse>,
        calls: Vec<RecordedCall>,
    }

    #[derive(Clone, Default)]
    struct FakeTransport {
        state: Arc<Mutex<FakeState>>,
    }

    impl FakeTransport {
        fn with_responses(responses: Vec<HttpResponse>) -> Self {
            let state = FakeState {
                responses: responses.into_iter().collect(),
                calls: Vec::new(),
            };
            Self {
                state: Arc::new(Mutex::new(state)),
            }
        }

        fn calls(&self) -> Vec<RecordedCall> {
            self.state.lock().unwrap().calls.clone()
        }

        fn record(
            &self,
            method: &'static str,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
            body: Option<&str>,
        ) -> Result<HttpResponse> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCall {
                method,
                path: path.to_owned(),
                query: query
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
                bearer: bearer.map(str::to_owned),
                body: body.map(str::to_owned),
            });
            state
                .responses
                .pop_front()
                .ok_or_else(|| OperatorError::Transport("no canned response".to_owned()))
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            self.record("GET", path, query, bearer, None)
        }
    }

    impl WriteTransport for FakeTransport {
        async fn post(
            &self,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
            json_body: Option<&str>,
        ) -> Result<HttpResponse> {
            self.record("POST", path, query, bearer, json_body)
        }

        async fn delete(
            &self,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            self.record("DELETE", path, query, bearer, None)
        }

        async fn put_bytes(
            &self,
            url: &str,
            content_type: &str,
            body: Vec<u8>,
        ) -> Result<HttpResponse> {
            // Record the absolute URL as the path, the content type as a query
            // pair, and the byte length as the body so assertions can inspect
            // the upload without depending on the (binary) payload.
            self.record(
                "PUT",
                url,
                &[("content-type", content_type.to_owned())],
                None,
                Some(&body.len().to_string()),
            )
        }
    }

    fn ok(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            body: body.to_owned(),
        }
    }

    fn authed(transport: FakeTransport) -> OperatorClient<FakeTransport, StaticTokenProvider> {
        OperatorClient::with_transport(transport, StaticTokenProvider::new("token-123"))
    }

    #[tokio::test]
    async fn status_hits_public_endpoint_and_decodes() {
        let transport = FakeTransport::with_responses(vec![ok(
            r#"{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}"#,
        )]);
        let client =
            OperatorClient::with_transport(transport.clone(), StaticTokenProvider::anonymous());

        let status = client.status().await.unwrap();

        assert_eq!(status.state, tbo_core::domain::BoothState::Idle);
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].path, "/v1/status");
        assert!(calls[0].query.is_empty());
        assert!(calls[0].bearer.is_none());
    }

    #[tokio::test]
    async fn messages_builds_filter_query_and_sends_bearer() {
        let transport = FakeTransport::with_responses(vec![ok(r#"{"items":[]}"#)]);
        let client = authed(transport.clone());

        let list = client
            .messages(Some(MessageStatus::Pending), None, Some(25))
            .await
            .unwrap();

        assert!(list.items.is_empty());
        let calls = transport.calls();
        assert_eq!(calls[0].path, "/v1/messages");
        assert_eq!(
            calls[0].query,
            vec![
                ("status".to_owned(), "pending".to_owned()),
                ("limit".to_owned(), "25".to_owned()),
            ]
        );
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
    }

    #[tokio::test]
    async fn questions_builds_cursor_and_status_query() {
        let transport =
            FakeTransport::with_responses(vec![ok(r#"{"items":[],"nextCursor":null}"#)]);
        let client = authed(transport.clone());

        client
            .questions(Some(QuestionStatus::Active), Some("cur-1"), Some(50))
            .await
            .unwrap();

        let calls = transport.calls();
        assert_eq!(calls[0].path, "/v1/questions");
        assert_eq!(
            calls[0].query,
            vec![
                ("status".to_owned(), "active".to_owned()),
                ("cursor".to_owned(), "cur-1".to_owned()),
                ("limit".to_owned(), "50".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn events_encodes_repeated_type_params() {
        let transport =
            FakeTransport::with_responses(vec![ok(r#"{"items":[],"nextCursor":null}"#)]);
        let client = authed(transport.clone());

        let filter = EventQuery {
            booth_id: Some("booth-a".to_owned()),
            types: vec!["call_started".to_owned(), "call_ended".to_owned()],
            limit: Some(100),
            ..EventQuery::default()
        };
        client.events(&filter).await.unwrap();

        let calls = transport.calls();
        assert_eq!(calls[0].path, "/v1/events");
        assert_eq!(
            calls[0].query,
            vec![
                ("boothId".to_owned(), "booth-a".to_owned()),
                ("type".to_owned(), "call_started".to_owned()),
                ("type".to_owned(), "call_ended".to_owned()),
                ("limit".to_owned(), "100".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn stats_overview_sends_window_and_bearer() {
        let body = r#"{"window":"7d","rangeEnd":"2026-01-01T00:00:00Z","generatedAt":"2026-01-01T00:00:00Z","timezone":"UTC","calls":{"total":3,"completed":2,"inProgress":0,"outcomes":{"recording_completed":2},"perDay":[]},"messages":{"total":2,"byStatus":{"pending":1}},"playback":{"totalPlaybacks":5},"pickupsHangups":{"pickups":3,"hangups":3,"digitsDialed":{"5":1}},"uploads":{"succeeded":2,"failed":0},"topQuestions":[],"hourly":[],"busiest":{},"boothBreakdown":[]}"#;
        let transport = FakeTransport::with_responses(vec![ok(body)]);
        let client = authed(transport.clone());

        let overview = client
            .stats_overview(Some(StatsWindow::Week))
            .await
            .unwrap();

        assert_eq!(overview.calls.total, 3);
        assert_eq!(overview.messages.total, 2);
        let calls = transport.calls();
        assert_eq!(calls[0].path, "/v1/stats/overview");
        assert_eq!(calls[0].query, vec![("window".to_owned(), "7d".to_owned())]);
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
    }

    #[tokio::test]
    async fn system_current_lists_snapshots_with_bearer() {
        let body = r#"{"items":[{"boothId":"booth-1","snapshot":{"cpu":{"usageRatio":0.5},"temperatureCelsius":48.5,"memory":{"totalBytes":1000,"usedBytes":400}},"receivedAt":"2026-01-01T00:00:00Z","version":"0.3.2"}]}"#;
        let transport = FakeTransport::with_responses(vec![ok(body)]);
        let client = authed(transport.clone());

        let list = client.system_current().await.unwrap();

        assert_eq!(list.items.len(), 1);
        let envelope = &list.items[0];
        assert_eq!(envelope.booth_id, "booth-1");
        assert_eq!(envelope.version.as_deref(), Some("0.3.2"));
        assert_eq!(
            envelope.snapshot.temperature_celsius,
            Some(48.5),
            "temperature should decode"
        );
        let calls = transport.calls();
        assert_eq!(calls[0].path, "/v1/system/current");
        assert!(calls[0].query.is_empty());
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
    }

    #[tokio::test]
    async fn requires_auth_when_signed_out() {
        let transport = FakeTransport::with_responses(vec![]);
        let client =
            OperatorClient::with_transport(transport.clone(), StaticTokenProvider::anonymous());

        let err = client.operator_me().await.unwrap_err();

        assert!(matches!(err, OperatorError::Unauthenticated));
        // No request should have been issued.
        assert!(transport.calls().is_empty());
    }

    #[tokio::test]
    async fn maps_not_found_and_unauthorized_statuses() {
        let transport = FakeTransport::with_responses(vec![
            HttpResponse {
                status: 404,
                body: String::new(),
            },
            HttpResponse {
                status: 401,
                body: String::new(),
            },
        ]);
        let client = authed(transport);

        let not_found = client.message("missing").await.unwrap_err();
        assert!(matches!(not_found, OperatorError::NotFound));

        let unauthorized = client.message("denied").await.unwrap_err();
        assert!(matches!(unauthorized, OperatorError::Unauthorized(401)));
    }

    fn message_body(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","status":"approved","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"https://example/{id}.flac","sha256":"{id}","durationMs":1000}}}}"#
        )
    }

    #[tokio::test]
    async fn decide_message_posts_decision_body() {
        let transport = FakeTransport::with_responses(vec![ok(&message_body("m1"))]);
        let client = authed(transport.clone());

        let message = client
            .decide_message(
                "m1",
                MessageDecisionKind::Approve,
                Some("looks good".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(message.id, "m1");
        assert_eq!(message.status, MessageStatus::Approved);
        let calls = transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/messages/m1/decision");
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
        let body = calls[0].body.as_deref().unwrap();
        assert_eq!(body, r#"{"decision":"approve","notes":"looks good"}"#);
    }

    #[tokio::test]
    async fn decide_message_omits_absent_notes() {
        let transport = FakeTransport::with_responses(vec![ok(&message_body("m1"))]);
        let client = authed(transport.clone());

        client
            .decide_message("m1", MessageDecisionKind::Reject, None)
            .await
            .unwrap();

        let calls = transport.calls();
        assert_eq!(calls[0].body.as_deref(), Some(r#"{"decision":"reject"}"#));
    }

    #[tokio::test]
    async fn submit_translation_posts_text_body() {
        let body = r#"{"id":"t1","messageId":"m1","provider":"openai","status":"succeeded","createdAt":"2026-01-01T00:00:00Z"}"#;
        let transport = FakeTransport::with_responses(vec![ok(body)]);
        let client = authed(transport.clone());

        let transcription = client
            .submit_translation("m1", "hello there".to_owned(), Some("en".to_owned()))
            .await
            .unwrap();

        assert_eq!(transcription.id, "t1");
        let calls = transport.calls();
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/messages/m1/translation");
        assert_eq!(
            calls[0].body.as_deref(),
            Some(r#"{"translatedText":"hello there","translatedLanguage":"en"}"#)
        );
    }

    #[tokio::test]
    async fn retranscribe_posts_without_body() {
        let body = r#"{"id":"t2","messageId":"m1","provider":"openai","status":"pending","createdAt":"2026-01-01T00:00:00Z"}"#;
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 202,
            body: body.to_owned(),
        }]);
        let client = authed(transport.clone());

        let transcription = client.retranscribe_message("m1").await.unwrap();

        assert_eq!(transcription.id, "t2");
        let calls = transport.calls();
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/messages/m1/transcribe");
        assert!(calls[0].body.is_none());
    }

    #[tokio::test]
    async fn delete_message_succeeds_on_no_content() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 204,
            body: String::new(),
        }]);
        let client = authed(transport.clone());

        client.delete_message("m1").await.unwrap();

        let calls = transport.calls();
        assert_eq!(calls[0].method, "DELETE");
        assert_eq!(calls[0].path, "/v1/messages/m1");
        assert!(calls[0].body.is_none());
    }

    #[tokio::test]
    async fn write_action_maps_conflict_status() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 409,
            body: r#"{"error":"message_not_decidable"}"#.to_owned(),
        }]);
        let client = authed(transport);

        let err = client
            .decide_message("m1", MessageDecisionKind::Approve, None)
            .await
            .unwrap_err();

        assert!(matches!(err, OperatorError::Http { status: 409, .. }));
    }

    #[tokio::test]
    async fn write_action_requires_auth_when_signed_out() {
        let transport = FakeTransport::with_responses(vec![]);
        let client =
            OperatorClient::with_transport(transport.clone(), StaticTokenProvider::anonymous());

        let err = client.delete_message("m1").await.unwrap_err();

        assert!(matches!(err, OperatorError::Unauthenticated));
        assert!(transport.calls().is_empty());
    }

    fn question_body(id: &str, status: &str) -> String {
        format!(
            r#"{{"id":"{id}","prompt":"Prompt {id}","status":"{status}","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"https://example/{id}.flac","sha256":"{id}","durationMs":1000}}}}"#
        )
    }

    #[tokio::test]
    async fn activate_question_posts_without_body() {
        let transport = FakeTransport::with_responses(vec![ok(&question_body("q1", "active"))]);
        let client = authed(transport.clone());

        let question = client.activate_question("q1").await.unwrap();

        assert_eq!(question.status, QuestionStatus::Active);
        let calls = transport.calls();
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/questions/q1/activate");
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
        assert!(calls[0].body.is_none());
    }

    #[tokio::test]
    async fn deactivate_question_posts_without_body() {
        let transport = FakeTransport::with_responses(vec![ok(&question_body("q1", "draft"))]);
        let client = authed(transport.clone());

        let question = client.deactivate_question("q1").await.unwrap();

        assert_eq!(question.status, QuestionStatus::Draft);
        let calls = transport.calls();
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/questions/q1/deactivate");
    }

    #[tokio::test]
    async fn archive_question_deletes_on_no_content() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 204,
            body: String::new(),
        }]);
        let client = authed(transport.clone());

        client.archive_question("q1").await.unwrap();

        let calls = transport.calls();
        assert_eq!(calls[0].method, "DELETE");
        assert_eq!(calls[0].path, "/v1/questions/q1");
    }

    #[tokio::test]
    async fn create_question_with_audio_runs_the_three_step_flow() {
        let slot = r#"{"uploadUrl":"https://blob.example/c/q.flac?sig=abc","blobName":"questions/ab/q.flac","expiresAt":"2026-01-01T00:00:00Z","audioFileId":"file-1"}"#;
        let transport = FakeTransport::with_responses(vec![
            ok(slot),
            HttpResponse {
                status: 201,
                body: String::new(),
            },
            HttpResponse {
                status: 201,
                body: question_body("q1", "draft"),
            },
        ]);
        let client = authed(transport.clone());

        let question = client
            .create_question_with_audio("New prompt?".to_owned(), b"flac-bytes".to_vec(), None)
            .await
            .unwrap();

        assert_eq!(question.id, "q1");
        let calls = transport.calls();
        assert_eq!(calls.len(), 3);

        // 1. Reserve a SAS slot for the bytes.
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/uploads/sas");
        let sas: UploadSasRequest =
            serde_json::from_str(calls[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(sas.kind, UploadSasKind::QuestionAudio);
        assert_eq!(sas.size_bytes, 10);
        assert_eq!(sas.content_type, "audio/flac");
        assert_eq!(sas.sha256.len(), 64);
        assert!(sas.sha256.bytes().all(|b| b.is_ascii_hexdigit()));

        // 2. PUT the bytes to the (absolute) blob URL with no bearer.
        assert_eq!(calls[1].method, "PUT");
        assert_eq!(calls[1].path, "https://blob.example/c/q.flac?sig=abc");
        assert!(calls[1].bearer.is_none());
        assert_eq!(calls[1].body.as_deref(), Some("10"));

        // 3. Create the question referencing the uploaded file.
        assert_eq!(calls[2].method, "POST");
        assert_eq!(calls[2].path, "/v1/questions");
        let create: QuestionCreate =
            serde_json::from_str(calls[2].body.as_deref().unwrap()).unwrap();
        assert_eq!(create.prompt, "New prompt?");
        assert_eq!(create.audio_file_id, "file-1");
        assert!(create.status.is_none());
    }

    #[tokio::test]
    async fn create_question_with_audio_errors_without_audio_file_id() {
        // A `message`-kind slot omits the audio file id; the question flow must
        // not proceed to an upload.
        let slot = r#"{"uploadUrl":"https://blob.example/c/q.flac?sig=abc","blobName":"messages/ab/q.flac","expiresAt":"2026-01-01T00:00:00Z"}"#;
        let transport = FakeTransport::with_responses(vec![ok(slot)]);
        let client = authed(transport.clone());

        let err = client
            .create_question_with_audio("Hi?".to_owned(), b"x".to_vec(), None)
            .await
            .unwrap_err();

        assert!(matches!(err, OperatorError::InvalidRequest(_)));
        assert_eq!(transport.calls().len(), 1, "must stop before uploading");
    }

    #[tokio::test]
    async fn api_tokens_decodes_the_bare_array() {
        let body = r#"[{"id":"t1","name":"ci","last4":"abcd","createdAt":"2026-01-01T00:00:00Z","expiresAt":null,"lastUsedAt":null,"revokedAt":null}]"#;
        let transport = FakeTransport::with_responses(vec![ok(body)]);
        let client = authed(transport.clone());

        let tokens = client.api_tokens().await.unwrap();

        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].id, "t1");
        assert_eq!(tokens[0].last4, "abcd");
        let calls = transport.calls();
        assert_eq!(calls[0].method, "GET");
        assert_eq!(calls[0].path, "/v1/api-tokens");
        assert!(calls[0].query.is_empty());
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
    }

    #[tokio::test]
    async fn api_token_usage_forwards_the_days_query() {
        let body = r#"[{"date":"2026-01-01","count":3}]"#;
        let transport = FakeTransport::with_responses(vec![ok(body)]);
        let client = authed(transport.clone());

        let buckets = client.api_token_usage("t1", Some(7)).await.unwrap();

        assert_eq!(
            buckets,
            vec![ApiTokenUsageBucket {
                date: "2026-01-01".to_owned(),
                count: 3,
            }]
        );
        let calls = transport.calls();
        assert_eq!(calls[0].path, "/v1/api-tokens/t1/usage");
        assert_eq!(calls[0].query, vec![("days".to_owned(), "7".to_owned())]);
    }

    #[tokio::test]
    async fn create_api_token_posts_name_and_returns_plaintext() {
        let created = r#"{"id":"t1","name":"ci","last4":"wxyz","createdAt":"2026-01-01T00:00:00Z","expiresAt":null,"plaintext":"tbk_secret_value"}"#;
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 201,
            body: created.to_owned(),
        }]);
        let client = authed(transport.clone());

        let token = client
            .create_api_token("ci".to_owned(), Some(30))
            .await
            .unwrap();

        assert_eq!(token.plaintext, "tbk_secret_value");
        let calls = transport.calls();
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].path, "/v1/api-tokens");
        let request: CreateApiTokenRequest =
            serde_json::from_str(calls[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(request.name, "ci");
        assert_eq!(request.expires_in_days, Some(30));
    }

    #[tokio::test]
    async fn create_api_token_omits_expiry_when_none() {
        let created = r#"{"id":"t1","name":"ci","last4":"wxyz","createdAt":"2026-01-01T00:00:00Z","expiresAt":null,"plaintext":"secret"}"#;
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 201,
            body: created.to_owned(),
        }]);
        let client = authed(transport.clone());

        client
            .create_api_token("ci".to_owned(), None)
            .await
            .unwrap();

        let body = transport.calls()[0].body.clone().unwrap();
        assert!(
            !body.contains("expiresInDays"),
            "a never-expiring token must omit the field, got {body}"
        );
    }

    #[tokio::test]
    async fn revoke_api_token_deletes_on_no_content() {
        let transport = FakeTransport::with_responses(vec![HttpResponse {
            status: 204,
            body: String::new(),
        }]);
        let client = authed(transport.clone());

        client.revoke_api_token("t1").await.unwrap();

        let calls = transport.calls();
        assert_eq!(calls[0].method, "DELETE");
        assert_eq!(calls[0].path, "/v1/api-tokens/t1");
        assert_eq!(calls[0].bearer.as_deref(), Some("token-123"));
    }

    /// An SSE transport that replays canned byte chunks and records the query.
    #[derive(Clone)]
    struct FakeSseTransport {
        chunks: Vec<Vec<u8>>,
        last_query: Arc<Mutex<Vec<(String, String)>>>,
    }

    impl FakeSseTransport {
        fn new(chunks: &[&str]) -> Self {
            Self {
                chunks: chunks.iter().map(|c| c.as_bytes().to_vec()).collect(),
                last_query: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn query(&self) -> Vec<(String, String)> {
            self.last_query.lock().unwrap().clone()
        }
    }

    impl HttpTransport for FakeSseTransport {
        async fn get(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(HttpResponse {
                status: 200,
                body: String::new(),
            })
        }
    }

    impl SseTransport for FakeSseTransport {
        async fn get_sse(
            &self,
            _path: &str,
            query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<crate::transport::ByteStream> {
            *self.last_query.lock().unwrap() = query
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect();
            let chunks = self.chunks.clone();
            Ok(futures::stream::iter(chunks.into_iter().map(Ok)).boxed())
        }
    }

    fn event_json(id: &str) -> String {
        format!(
            r#"{{"id":"{id}","eventId":"{id}","boothId":"booth-1","bootId":"boot-1","type":"call_started","occurredAt":"2026-01-01T00:00:00Z","receivedAt":"2026-01-01T00:00:01Z"}}"#
        )
    }

    #[tokio::test]
    async fn events_stream_decodes_booth_events_and_skips_ready_and_ping() {
        let frame = format!(
            "event: ready\ndata: ok\n\nid: evt-1\nevent: booth-event\ndata: {}\n\nevent: ping\ndata: t\n\n",
            event_json("evt-1")
        );
        let transport = FakeSseTransport::new(&[&frame]);
        let client =
            OperatorClient::with_transport(transport.clone(), StaticTokenProvider::new("tok"));
        let filter = EventQuery {
            booth_id: Some("booth-1".to_owned()),
            types: vec!["call_started".to_owned()],
            ..EventQuery::default()
        };

        let mut stream = client.events_stream(&filter).await.unwrap();
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.id, "evt-1");
        assert!(stream.next().await.is_none());

        let query = transport.query();
        assert!(query.iter().any(|(k, v)| k == "boothId" && v == "booth-1"));
        assert!(
            query
                .iter()
                .any(|(k, v)| k == "type" && v == "call_started")
        );
    }

    #[tokio::test]
    async fn events_stream_requires_authentication() {
        let transport = FakeSseTransport::new(&[]);
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::anonymous());
        let result = client.events_stream(&EventQuery::default()).await;
        assert!(matches!(result, Err(OperatorError::Unauthenticated)));
    }
}
