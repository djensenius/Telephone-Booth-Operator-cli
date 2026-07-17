//! Background operator-identity tracking: admin gating and session
//! revalidation.
//!
//! On a fixed cadence (and once at startup) this controller re-fetches
//! `GET /v1/auth/me`. It caches whether the operator is an admin (used to gate
//! question-management actions) and, crucially, detects when the session has
//! become invalid server-side — e.g. the account was deleted or removed from
//! the operator group in Authentik. A revoked session is surfaced so the app
//! can sign the user out immediately rather than trusting a still-cached token.

use std::time::{Duration, Instant};

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_core::domain::OperatorMe;
use tbo_operator_client::{
    HttpTransport, OperatorClient, OperatorError, ReqwestTransport, TokenProvider,
};

use crate::data::SessionTokenProvider;

/// How often the signed-in operator's identity is re-validated.
const REVALIDATE_INTERVAL: Duration = Duration::from_mins(1);

/// The outcome of a single identity re-fetch.
enum Fetched {
    /// The operator is still valid; carries the fresh profile.
    Profile(Box<OperatorMe>),
    /// The session is no longer valid (account deleted or de-authorized).
    Revoked,
    /// A transient error (network, server) — leave cached state untouched.
    Transient,
}

/// Tracks the signed-in operator's identity off the UI thread.
///
/// `tick` re-validates on a fixed cadence while signed in; the app reads
/// [`IdentityController::is_admin`] to gate admin-only actions and calls
/// [`IdentityController::take_revoked`] to react to a server-side sign-out.
pub struct IdentityController<T = ReqwestTransport, A = SessionTokenProvider>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client: OperatorClient<T, A>,
    profile: Option<OperatorMe>,
    rx: Option<UnboundedReceiver<Fetched>>,
    in_flight: bool,
    last_refresh: Option<Instant>,
    revoked: bool,
}

impl<T, A> IdentityController<T, A>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Build a controller over the given operator client.
    pub fn new(client: OperatorClient<T, A>) -> Self {
        Self {
            client,
            profile: None,
            rx: None,
            in_flight: false,
            last_refresh: None,
            revoked: false,
        }
    }

    /// Whether the signed-in operator is a known administrator.
    ///
    /// Returns `false` until the first successful re-validation completes, so
    /// admin-only actions fail closed while permissions are still unknown.
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.profile.as_ref().is_some_and(|me| me.is_admin)
    }

    /// Whether the operator's identity has been loaded at least once.
    #[must_use]
    pub fn is_known(&self) -> bool {
        self.profile.is_some()
    }

    /// Consume a pending "session revoked" signal, if any.
    ///
    /// Returns `true` exactly once after a re-validation determines the session
    /// is no longer valid, so the app can sign out and reset local state.
    pub fn take_revoked(&mut self) -> bool {
        std::mem::take(&mut self.revoked)
    }

    /// Forget the cached identity (e.g. after a local sign-out) so the next
    /// sign-in re-fetches from scratch.
    pub fn reset(&mut self) {
        self.profile = None;
        self.last_refresh = None;
        self.revoked = false;
    }

    /// Trigger an identity re-fetch unless one is already in flight.
    pub fn refresh(&mut self) {
        if self.in_flight {
            return;
        }
        self.in_flight = true;
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let client = self.client.clone();
        tokio::spawn(async move {
            let outcome = match client.operator_me().await {
                Ok(me) => Fetched::Profile(Box::new(me)),
                Err(OperatorError::Unauthorized(_) | OperatorError::Unauthenticated) => {
                    Fetched::Revoked
                }
                Err(_) => Fetched::Transient,
            };
            let _ = tx.send(outcome);
        });
    }

    /// Advance the controller: apply results, then re-validate on cadence while
    /// `signed_in`.
    pub fn tick(&mut self, signed_in: bool) {
        self.drain();
        if signed_in {
            if self.is_due(Instant::now()) {
                self.refresh();
            }
        } else if self.profile.is_some() {
            // Signed out locally: drop stale identity so a later sign-in is
            // re-validated before any admin action is allowed.
            self.reset();
        }
    }

    /// Apply any completed fetch (non-blocking).
    fn drain(&mut self) {
        loop {
            let Some(rx) = self.rx.as_mut() else {
                return;
            };
            match rx.try_recv() {
                Ok(outcome) => self.apply(outcome),
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.rx = None;
                    self.in_flight = false;
                    return;
                }
            }
        }
    }

    /// Apply a single fetch outcome.
    fn apply(&mut self, outcome: Fetched) {
        self.in_flight = false;
        self.last_refresh = Some(Instant::now());
        self.rx = None;
        match outcome {
            Fetched::Profile(me) => self.profile = Some(*me),
            Fetched::Revoked => {
                self.profile = None;
                self.revoked = true;
            }
            Fetched::Transient => {}
        }
    }

    /// Whether a re-validation is due at `now`.
    fn is_due(&self, now: Instant) -> bool {
        if self.in_flight {
            return false;
        }
        self.last_refresh
            .is_none_or(|last| now.duration_since(last) >= REVALIDATE_INTERVAL)
    }

    /// Await and apply the next pending result (test helper).
    #[cfg(test)]
    async fn recv_once(&mut self) {
        if let Some(rx) = self.rx.as_mut()
            && let Some(outcome) = rx.recv().await
        {
            self.apply(outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use tbo_operator_client::{HttpResponse, HttpTransport, Result, StaticTokenProvider};

    use super::*;

    #[derive(Clone)]
    struct FakeTransport {
        response: Arc<Mutex<HttpResponse>>,
    }

    impl FakeTransport {
        fn new(status: u16, body: &str) -> Self {
            Self {
                response: Arc::new(Mutex::new(HttpResponse {
                    status,
                    body: body.to_owned(),
                })),
            }
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(self.response.lock().unwrap().clone())
        }
    }

    fn controller(
        status: u16,
        body: &str,
    ) -> (
        IdentityController<FakeTransport, StaticTokenProvider>,
        Arc<Mutex<HttpResponse>>,
    ) {
        let transport = FakeTransport::new(status, body);
        let handle = Arc::clone(&transport.response);
        let client =
            OperatorClient::with_transport(transport, StaticTokenProvider::new("token-123"));
        (IdentityController::new(client), handle)
    }

    const ADMIN_ME: &str = r#"{"id":"u1","email":"a@example.com","name":"Admin","groups":["ops","admins"],"isAdmin":true,"providerName":"authentik"}"#;
    const OPERATOR_ME: &str = r#"{"id":"u2","email":"o@example.com","name":"Op","groups":["ops"],"isAdmin":false,"providerName":"authentik"}"#;

    #[tokio::test]
    async fn admin_profile_marks_is_admin() {
        let (mut controller, _handle) = controller(200, ADMIN_ME);
        controller.refresh();
        controller.recv_once().await;
        assert!(controller.is_admin());
        assert!(controller.is_known());
        assert!(!controller.take_revoked());
    }

    #[tokio::test]
    async fn non_admin_profile_is_not_admin() {
        let (mut controller, _handle) = controller(200, OPERATOR_ME);
        controller.refresh();
        controller.recv_once().await;
        assert!(controller.is_known());
        assert!(!controller.is_admin());
    }

    #[tokio::test]
    async fn unauthorized_marks_session_revoked() {
        let (mut controller, handle) = controller(200, ADMIN_ME);
        controller.refresh();
        controller.recv_once().await;
        assert!(controller.is_admin());

        // The account was deleted server-side: /auth/me now returns 401.
        *handle.lock().unwrap() = HttpResponse {
            status: 401,
            body: String::new(),
        };
        controller.last_refresh = None; // allow an immediate re-validation
        controller.refresh();
        controller.recv_once().await;

        assert!(
            controller.take_revoked(),
            "revocation should be signalled once"
        );
        assert!(!controller.take_revoked(), "signal is consumed");
        assert!(!controller.is_admin());
        assert!(!controller.is_known());
    }

    #[tokio::test]
    async fn transient_error_keeps_cached_identity() {
        let (mut controller, handle) = controller(200, ADMIN_ME);
        controller.refresh();
        controller.recv_once().await;

        *handle.lock().unwrap() = HttpResponse {
            status: 500,
            body: "boom".to_owned(),
        };
        controller.last_refresh = None;
        controller.refresh();
        controller.recv_once().await;

        assert!(
            controller.is_admin(),
            "transient failure must not drop admin"
        );
        assert!(!controller.take_revoked());
    }

    #[test]
    fn is_due_respects_interval_and_in_flight() {
        let (mut controller, _handle) = controller(200, ADMIN_ME);
        let now = Instant::now();
        assert!(controller.is_due(now));
        controller.last_refresh = Some(now);
        assert!(!controller.is_due(now));
        assert!(controller.is_due(now + REVALIDATE_INTERVAL));
        controller.in_flight = true;
        assert!(!controller.is_due(now + REVALIDATE_INTERVAL));
    }
}
