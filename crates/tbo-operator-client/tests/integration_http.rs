//! End-to-end integration tests for [`OperatorClient`] against a mock HTTP
//! server.
//!
//! The in-crate unit tests exercise request construction through an in-memory
//! `FakeTransport`. These tests instead drive the **real** `ReqwestTransport`
//! (via [`OperatorClient::new`]) against a [`wiremock`] server, covering the
//! parts the fake cannot: base-URL joining, bearer-header injection, query
//! encoding, HTTP status mapping, and JSON / server-sent-event decoding over an
//! actual socket.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use futures::StreamExt;
use tbo_core::domain::{BoothState, MessageDecisionKind, MessageStatus};
use tbo_operator_client::{
    EventQuery, OperatorClient, OperatorError, ReqwestTransport, StaticTokenProvider,
};
use wiremock::matchers::{body_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "test-bearer-token";

/// JSON body for a single approved `Message`.
fn message_body(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","status":"approved","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"https://example/{id}.flac","sha256":"{id}","durationMs":1000}}}}"#
    )
}

/// Build a client whose transport is rooted at the mock server and that sends
/// [`TOKEN`] as its bearer credential.
fn client(server: &MockServer) -> OperatorClient<ReqwestTransport> {
    OperatorClient::new(server.uri(), StaticTokenProvider::new(TOKEN))
        .expect("client builds against the mock server")
}

#[tokio::test]
async fn status_decodes_over_real_transport() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/status"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}"#),
        )
        .expect(1)
        .mount(&server)
        .await;

    let status = client(&server).status().await.unwrap();

    assert_eq!(status.state, BoothState::Idle);
}

#[tokio::test]
async fn messages_forwards_filter_query_and_bearer() {
    let server = MockServer::start().await;
    // The mock only matches when the query string and Authorization header are
    // exactly what the client should have produced; an unmatched request makes
    // wiremock answer `404`, which would fail the decode below.
    Mock::given(method("GET"))
        .and(path("/v1/messages"))
        .and(query_param("status", "pending"))
        .and(query_param("limit", "25"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"items":[]}"#))
        .expect(1)
        .mount(&server)
        .await;

    let list = client(&server)
        .messages(Some(MessageStatus::Pending), None, Some(25))
        .await
        .unwrap();

    assert!(list.items.is_empty());
}

#[tokio::test]
async fn not_found_maps_to_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/messages/missing"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client(&server).message("missing").await.unwrap_err();

    assert!(matches!(err, OperatorError::NotFound));
}

#[tokio::test]
async fn unauthorized_status_maps_to_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/status/history"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let err = client(&server)
        .status_history(None, Some(10))
        .await
        .unwrap_err();

    assert!(matches!(err, OperatorError::Unauthorized(401)));
}

#[tokio::test]
async fn decide_message_posts_json_body_and_decodes_response() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/m1/decision"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .and(body_json(serde_json::json!({
            "decision": "approve",
            "notes": "looks good",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(message_body("m1")))
        .expect(1)
        .mount(&server)
        .await;

    let message = client(&server)
        .decide_message(
            "m1",
            MessageDecisionKind::Approve,
            Some("looks good".to_owned()),
        )
        .await
        .unwrap();

    assert_eq!(message.id, "m1");
    assert_eq!(message.status, MessageStatus::Approved);
}

#[tokio::test]
async fn delete_message_succeeds_on_no_content() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/v1/messages/m1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client(&server).delete_message("m1").await.unwrap();
}

#[tokio::test]
async fn create_api_token_decodes_one_time_secret() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/api-tokens"))
        .and(body_json(serde_json::json!({ "name": "ci" })))
        .respond_with(ResponseTemplate::new(201).set_body_string(
            r#"{"id":"tok_1","name":"ci","last4":"wxyz","createdAt":"2026-01-01T00:00:00Z","plaintext":"secret-once"}"#,
        ))
        .expect(1)
        .mount(&server)
        .await;

    let created = client(&server)
        .create_api_token("ci".to_owned(), None)
        .await
        .unwrap();

    assert_eq!(created.id, "tok_1");
    assert_eq!(created.plaintext, "secret-once");
}

#[tokio::test]
async fn events_stream_decodes_server_sent_events() {
    let server = MockServer::start().await;
    let event = r#"{"id":"evt-1","eventId":"evt-1","boothId":"booth-1","bootId":"boot-1","type":"call_started","occurredAt":"2026-01-01T00:00:00Z","receivedAt":"2026-01-01T00:00:01Z"}"#;
    // A `ready` handshake and `ping` heartbeat bracket the real frame; both must
    // be transparently skipped, leaving exactly one decoded event.
    let body = format!(
        "event: ready\ndata: ok\n\nid: evt-1\nevent: booth-event\ndata: {event}\n\nevent: ping\ndata: t\n\n"
    );
    Mock::given(method("GET"))
        .and(path("/v1/events/stream"))
        .and(query_param("boothId", "booth-1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;

    let filter = EventQuery {
        booth_id: Some("booth-1".to_owned()),
        ..EventQuery::default()
    };
    let mut stream = client(&server).events_stream(&filter).await.unwrap();

    let first = stream.next().await.unwrap().unwrap();
    assert_eq!(first.id, "evt-1");
    assert!(stream.next().await.is_none());
}
