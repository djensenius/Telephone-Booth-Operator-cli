//! Device-code flow tests driven by an in-memory fake transport.
//!
//! These exercise the OAuth status/error mapping and the polling loop without a
//! network or a mock HTTP server.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unnecessary_wraps)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tbo_auth::{
    AuthClient, AuthError, DeviceAuthorization, DeviceTokenOutcome, HttpResponse, HttpTransport,
    Result,
};
use tbo_core::config::AuthConfig;

/// A recorded outbound request.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RecordedRequest {
    url: String,
    form: Vec<(String, String)>,
}

/// Transport that returns queued responses and records the requests it saw.
///
/// State is shared behind `Arc` so a handle can be cloned before the transport
/// is moved into the client, letting tests inspect recorded requests.
#[derive(Clone)]
struct FakeTransport {
    responses: Arc<Mutex<VecDeque<Result<HttpResponse>>>>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

impl FakeTransport {
    fn new(responses: Vec<Result<HttpResponse>>) -> Self {
        Self {
            responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl HttpTransport for FakeTransport {
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse> {
        self.requests.lock().unwrap().push(RecordedRequest {
            url: url.to_owned(),
            form: form
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        });
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("FakeTransport ran out of queued responses")
    }
}

fn ok(status: u16, body: &str) -> Result<HttpResponse> {
    Ok(HttpResponse {
        status,
        body: body.to_owned(),
    })
}

fn test_config() -> AuthConfig {
    AuthConfig {
        issuer: "https://auth.example/application/o/test-app".to_owned(),
        client_id: "client-123".to_owned(),
        scopes: "openid offline_access".to_owned(),
    }
}

fn client_with(transport: FakeTransport) -> AuthClient<FakeTransport> {
    AuthClient::with_transport(transport, &test_config())
}

fn authorization_with_interval(interval: i64) -> DeviceAuthorization {
    DeviceAuthorization {
        device_code: "dc".to_owned(),
        user_code: "U".to_owned(),
        verification_uri: "https://auth.example/device".to_owned(),
        verification_uri_complete: None,
        expires_in: 600,
        interval,
    }
}

#[tokio::test]
async fn begin_device_authorization_posts_to_device_endpoint() {
    let transport = FakeTransport::new(vec![ok(
        200,
        r#"{
            "device_code": "dc",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://auth.example/device",
            "expires_in": 600,
            "interval": 5
        }"#,
    )]);
    let handle = transport.clone();
    let client = client_with(transport);

    let authorization = client.begin_device_authorization().await.unwrap();

    assert_eq!(authorization.user_code, "ABCD-EFGH");
    assert_eq!(authorization.device_code, "dc");

    let requests = handle.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].url.ends_with("/device/"));
    assert!(
        requests[0]
            .form
            .contains(&("client_id".to_owned(), "client-123".to_owned()))
    );
    assert!(
        requests[0]
            .form
            .contains(&("scope".to_owned(), "openid offline_access".to_owned()))
    );
}

#[tokio::test]
async fn begin_device_authorization_maps_failure() {
    let transport = FakeTransport::new(vec![ok(400, r#"{"error":"invalid_client"}"#)]);
    let client = client_with(transport);

    let error = client.begin_device_authorization().await.unwrap_err();

    assert!(matches!(error, AuthError::DeviceAuthorizationFailed(_)));
}

#[tokio::test]
async fn exchange_device_code_maps_oauth_states() {
    let transport = FakeTransport::new(vec![
        ok(400, r#"{"error":"authorization_pending"}"#),
        ok(400, r#"{"error":"slow_down"}"#),
        ok(400, r#"{"error":"access_denied"}"#),
        ok(400, r#"{"error":"expired_token"}"#),
        ok(400, r#"{"error":"server_error"}"#),
        ok(
            200,
            r#"{"access_token":"at","refresh_token":"rt","expires_in":300}"#,
        ),
    ]);
    let handle = transport.clone();
    let client = client_with(transport);

    assert_eq!(
        client.exchange_device_code("dc").await.unwrap(),
        DeviceTokenOutcome::Pending
    );
    assert_eq!(
        client.exchange_device_code("dc").await.unwrap(),
        DeviceTokenOutcome::SlowDown
    );
    assert_eq!(
        client.exchange_device_code("dc").await.unwrap(),
        DeviceTokenOutcome::Denied
    );
    assert_eq!(
        client.exchange_device_code("dc").await.unwrap(),
        DeviceTokenOutcome::Expired
    );
    assert_eq!(
        client.exchange_device_code("dc").await.unwrap(),
        DeviceTokenOutcome::Error("server_error".to_owned())
    );
    match client.exchange_device_code("dc").await.unwrap() {
        DeviceTokenOutcome::Tokens(tokens) => {
            assert_eq!(tokens.access_token, "at");
            assert_eq!(tokens.refresh_token.as_deref(), Some("rt"));
        }
        other => panic!("expected tokens, got {other:?}"),
    }

    let requests = handle.requests();
    assert_eq!(requests.len(), 6);
    assert!(requests[0].url.ends_with("/token/"));
    assert!(requests[0].form.contains(&(
        "grant_type".to_owned(),
        "urn:ietf:params:oauth:grant-type:device_code".to_owned()
    )));
}

#[tokio::test]
async fn exchange_device_code_treats_network_failure_as_transient() {
    let transport = FakeTransport::new(vec![Err(AuthError::Transport(
        "connection reset".to_owned(),
    ))]);
    let client = client_with(transport);

    let error = client.exchange_device_code("dc").await.unwrap_err();

    assert!(matches!(error, AuthError::Transient(_)));
}

#[tokio::test]
async fn poll_for_token_succeeds_after_pending_and_slow_down() {
    tokio::time::pause();
    let transport = FakeTransport::new(vec![
        ok(400, r#"{"error":"authorization_pending"}"#),
        ok(400, r#"{"error":"slow_down"}"#),
        Err(AuthError::Transport("blip".to_owned())),
        ok(200, r#"{"access_token":"final","expires_in":300}"#),
    ]);
    let client = client_with(transport);

    let tokens = client
        .poll_for_token(&authorization_with_interval(1))
        .await
        .unwrap();

    assert_eq!(tokens.access_token, "final");
}

#[tokio::test]
async fn poll_for_token_surfaces_denied() {
    tokio::time::pause();
    let transport = FakeTransport::new(vec![ok(400, r#"{"error":"access_denied"}"#)]);
    let client = client_with(transport);

    let error = client
        .poll_for_token(&authorization_with_interval(1))
        .await
        .unwrap_err();

    assert!(matches!(error, AuthError::AccessDenied));
}

#[tokio::test]
async fn poll_for_token_expires_at_deadline() {
    tokio::time::pause();
    // Only ever returns "pending"; the loop must stop at the device-code
    // deadline. This relies on the deadline using `tokio::time::Instant` so it
    // advances together with the virtualized `sleep`.
    let transport = FakeTransport::new(vec![
        ok(400, r#"{"error":"authorization_pending"}"#),
        ok(400, r#"{"error":"authorization_pending"}"#),
        ok(400, r#"{"error":"authorization_pending"}"#),
        ok(400, r#"{"error":"authorization_pending"}"#),
        ok(400, r#"{"error":"authorization_pending"}"#),
    ]);
    let client = client_with(transport);
    let authorization = DeviceAuthorization {
        device_code: "dc".to_owned(),
        user_code: "U".to_owned(),
        verification_uri: "https://auth.example/device".to_owned(),
        verification_uri_complete: None,
        expires_in: 3,
        interval: 1,
    };

    let error = client.poll_for_token(&authorization).await.unwrap_err();

    assert!(matches!(error, AuthError::ExpiredToken));
}

#[tokio::test]
async fn refresh_returns_tokens_on_success() {
    let transport = FakeTransport::new(vec![ok(
        200,
        r#"{"access_token":"new","refresh_token":"newrt","expires_in":300}"#,
    )]);
    let handle = transport.clone();
    let client = client_with(transport);

    let tokens = client.refresh("old-rt").await.unwrap();

    assert_eq!(tokens.access_token, "new");
    assert_eq!(tokens.refresh_token.as_deref(), Some("newrt"));

    let requests = handle.requests();
    assert!(
        requests[0]
            .form
            .contains(&("grant_type".to_owned(), "refresh_token".to_owned()))
    );
    assert!(
        requests[0]
            .form
            .contains(&("refresh_token".to_owned(), "old-rt".to_owned()))
    );
}

#[tokio::test]
async fn refresh_client_error_invalidates_token() {
    let transport = FakeTransport::new(vec![ok(400, r#"{"error":"invalid_grant"}"#)]);
    let client = client_with(transport);

    let error = client.refresh("old-rt").await.unwrap_err();

    assert!(matches!(error, AuthError::RefreshTokenInvalid(_)));
}

#[tokio::test]
async fn refresh_server_error_is_transient() {
    let transport = FakeTransport::new(vec![ok(503, "service unavailable")]);
    let client = client_with(transport);

    let error = client.refresh("old-rt").await.unwrap_err();

    assert!(matches!(error, AuthError::Transient(_)));
}
