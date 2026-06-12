//! The operator API client: typed read endpoints over an [`HttpTransport`].

use serde::de::DeserializeOwned;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use tbo_core::domain::{
    BoothEventList, BoothStatus, CallSessionDetail, CallSessionList, Message, MessageList,
    MessageStatus, OperatorMe, QuestionList, QuestionStatus, StatusHistory, TranscriptionList,
};

use crate::error::{OperatorError, Result};
use crate::token::{StaticTokenProvider, TokenProvider};
use crate::transport::{HttpTransport, ReqwestTransport};

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
        match response.status {
            _ if response.is_success() => serde_json::from_str(&response.body)
                .map_err(|err| OperatorError::Decode(err.to_string())),
            401 | 403 => Err(OperatorError::Unauthorized(response.status)),
            404 => Err(OperatorError::NotFound),
            status => Err(OperatorError::Http {
                status,
                body: response.body,
            }),
        }
    }
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
        path: String,
        query: Vec<(String, String)>,
        bearer: Option<String>,
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
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            path: &str,
            query: &[(&str, String)],
            bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            let mut state = self.state.lock().unwrap();
            state.calls.push(RecordedCall {
                path: path.to_owned(),
                query: query
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
                bearer: bearer.map(str::to_owned),
            });
            state
                .responses
                .pop_front()
                .ok_or_else(|| OperatorError::Transport("no canned response".to_owned()))
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
}
