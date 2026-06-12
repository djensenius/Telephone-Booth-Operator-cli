//! Session-manager orchestration tests using an in-memory store and a fake
//! transport, with an explicit `now` so token expiry needs no clock mocking.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::unnecessary_wraps)]

use std::collections::VecDeque;
use std::sync::Mutex;

use tbo_auth::{
    AuthClient, AuthError, HttpResponse, HttpTransport, InMemoryTokenStore, Result, SessionManager,
    StoredSession, TokenStore,
};
use tbo_core::config::AuthConfig;
use time::{Duration, OffsetDateTime};

/// Transport that returns queued responses for refresh calls.
struct FakeTransport {
    responses: Mutex<VecDeque<Result<HttpResponse>>>,
    calls: Mutex<usize>,
}

impl FakeTransport {
    fn new(responses: Vec<Result<HttpResponse>>) -> Self {
        Self {
            responses: Mutex::new(responses.into_iter().collect()),
            calls: Mutex::new(0),
        }
    }
}

impl HttpTransport for FakeTransport {
    async fn post_form(&self, _url: &str, _form: &[(&str, &str)]) -> Result<HttpResponse> {
        *self.calls.lock().unwrap() += 1;
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

fn config() -> AuthConfig {
    AuthConfig {
        issuer: "https://auth.example/application/o/test-app".to_owned(),
        client_id: "client-123".to_owned(),
        scopes: "openid offline_access".to_owned(),
    }
}

fn manager(
    responses: Vec<Result<HttpResponse>>,
    store: InMemoryTokenStore,
) -> SessionManager<InMemoryTokenStore, FakeTransport> {
    let client = AuthClient::with_transport(FakeTransport::new(responses), &config());
    SessionManager::with_client(client, store)
}

fn seed(store: &InMemoryTokenStore, refresh: Option<&str>, expires_at: OffsetDateTime) {
    store
        .save(&StoredSession {
            access_token: "old-access".to_owned(),
            refresh_token: refresh.map(str::to_owned),
            id_token: Some("old-id".to_owned()),
            expires_at: Some(expires_at),
        })
        .unwrap();
}

#[tokio::test]
async fn returns_none_when_signed_out() {
    let manager = manager(vec![], InMemoryTokenStore::new());
    assert_eq!(
        manager
            .access_token(OffsetDateTime::now_utc())
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn returns_cached_token_when_fresh() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    seed(&store, Some("rt"), now + Duration::minutes(30));
    // No queued responses: a refresh here would panic, proving none happens.
    let manager = manager(vec![], store);

    assert_eq!(
        manager.access_token(now).await.unwrap(),
        Some("old-access".to_owned())
    );
}

#[tokio::test]
async fn refreshes_when_expiring_soon_and_retains_refresh_token() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    seed(&store, Some("rt"), now + Duration::seconds(30));
    // Response omits refresh_token, so the existing one must be retained.
    let manager = manager(
        vec![ok(
            200,
            r#"{"access_token":"fresh-access","expires_in":300}"#,
        )],
        store,
    );

    let token = manager.access_token(now).await.unwrap();
    assert_eq!(token, Some("fresh-access".to_owned()));

    let stored = manager.current_session().unwrap().unwrap();
    assert_eq!(stored.access_token, "fresh-access");
    assert_eq!(stored.refresh_token.as_deref(), Some("rt"));
    assert_eq!(stored.id_token.as_deref(), Some("old-id"));
    assert_eq!(stored.expires_at, Some(now + Duration::seconds(300)));
}

#[tokio::test]
async fn rotates_refresh_token_when_provider_returns_a_new_one() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    seed(&store, Some("rt"), now + Duration::seconds(30));
    let manager = manager(
        vec![ok(
            200,
            r#"{"access_token":"fresh","refresh_token":"rotated","expires_in":300}"#,
        )],
        store,
    );

    manager.access_token(now).await.unwrap();

    let stored = manager.current_session().unwrap().unwrap();
    assert_eq!(stored.refresh_token.as_deref(), Some("rotated"));
}

#[tokio::test]
async fn invalid_refresh_token_clears_session() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    seed(&store, Some("rt"), now + Duration::seconds(30));
    let manager = manager(vec![ok(400, r#"{"error":"invalid_grant"}"#)], store);

    let error = manager.access_token(now).await.unwrap_err();
    assert!(matches!(error, AuthError::RefreshTokenInvalid(_)));
    assert_eq!(manager.current_session().unwrap(), None);
}

#[tokio::test]
async fn transient_refresh_falls_back_to_still_valid_token() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    // Within the refresh margin but not yet expired.
    seed(&store, Some("rt"), now + Duration::seconds(30));
    let manager = manager(vec![ok(503, "unavailable")], store);

    let token = manager.access_token(now).await.unwrap();
    assert_eq!(token, Some("old-access".to_owned()));
    // Session is preserved for a later retry.
    assert!(manager.current_session().unwrap().is_some());
}

#[tokio::test]
async fn transient_refresh_errors_when_already_expired() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    seed(&store, Some("rt"), now - Duration::seconds(10));
    let manager = manager(vec![ok(503, "unavailable")], store);

    let error = manager.access_token(now).await.unwrap_err();
    assert!(matches!(error, AuthError::Transient(_)));
}

#[tokio::test]
async fn expired_token_without_refresh_token_signs_out() {
    let now = OffsetDateTime::now_utc();
    let store = InMemoryTokenStore::new();
    seed(&store, None, now - Duration::seconds(10));
    let manager = manager(vec![], store);

    assert_eq!(manager.access_token(now).await.unwrap(), None);
    assert_eq!(manager.current_session().unwrap(), None);
}

#[tokio::test]
async fn complete_login_then_sign_out() {
    let now = OffsetDateTime::now_utc();
    let manager = manager(vec![], InMemoryTokenStore::new());
    let tokens = serde_json::from_str(
        r#"{"access_token":"at","refresh_token":"rt","id_token":"it","expires_in":300}"#,
    )
    .unwrap();

    let session = manager.complete_login(&tokens, now).unwrap();
    assert_eq!(session.access_token, "at");
    assert!(manager.is_signed_in().unwrap());
    assert_eq!(session.expires_at, Some(now + Duration::seconds(300)));

    manager.sign_out().unwrap();
    assert!(!manager.is_signed_in().unwrap());
}
