//! Interactive Authentik device-code login, driven off the UI thread.
//!
//! [`AuthController`] owns the [`SessionManager`] and runs the device flow in a
//! background task: it begins device authorization, surfaces the user code and
//! verification URL for display, polls until the user approves, and persists
//! the resulting session. The UI thread polls [`AuthController::drain`] each
//! tick to advance the visible [`AuthPhase`] without blocking rendering.

use std::sync::Arc;

use time::OffsetDateTime;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::task::JoinHandle;

use tbo_auth::{HttpTransport, ReqwestTransport, SessionManager, TokenStore};

use crate::ui::toast::Toasts;

/// The device-authorization details shown to the user while awaiting approval.
#[derive(Debug, Clone)]
pub struct PendingLogin {
    /// Short code the user enters at the verification URL.
    pub user_code: String,
    /// URL the user opens to enter the code.
    pub verification_uri: String,
    /// URL embedding the code for one-tap verification, when provided.
    pub verification_uri_complete: Option<String>,
}

/// The user-visible authentication state.
#[derive(Debug, Clone)]
pub enum AuthPhase {
    /// No stored session.
    SignedOut,
    /// A device-authorization request is in flight.
    Starting,
    /// Waiting for the user to approve the displayed code.
    AwaitingApproval(PendingLogin),
    /// A session is stored; the optional access-token expiry is shown.
    SignedIn {
        /// Absolute expiry of the current access token, when known.
        expires_at: Option<OffsetDateTime>,
    },
    /// The last login attempt failed.
    Failed(String),
}

/// A message from the background login task to the controller.
enum AuthOutcome {
    /// Device authorization succeeded; display the code and keep waiting.
    Pending(PendingLogin),
    /// The user approved and the session was stored.
    Completed {
        /// Access-token expiry of the stored session.
        expires_at: Option<OffsetDateTime>,
    },
    /// The login attempt failed.
    Failed(String),
}

/// The type-erased session manager shared with the background login task.
type SharedManager<T> = Arc<SessionManager<Box<dyn TokenStore>, T>>;

/// Drives the device-code login flow and tracks the visible auth phase.
pub struct AuthController<T: HttpTransport = ReqwestTransport> {
    manager: SharedManager<T>,
    phase: AuthPhase,
    rx: Option<UnboundedReceiver<AuthOutcome>>,
    task: Option<JoinHandle<()>>,
}

impl<T: HttpTransport + 'static> AuthController<T> {
    /// Build the controller, deriving the initial phase from any stored
    /// session.
    ///
    /// # Errors
    /// Returns an error if the stored session cannot be read.
    pub fn new(manager: SharedManager<T>) -> tbo_auth::Result<Self> {
        let phase = match manager.current_session()? {
            Some(session) => AuthPhase::SignedIn {
                expires_at: session.expires_at,
            },
            None => AuthPhase::SignedOut,
        };
        Ok(Self {
            manager,
            phase,
            rx: None,
            task: None,
        })
    }

    /// The current user-visible phase.
    #[must_use]
    pub fn phase(&self) -> &AuthPhase {
        &self.phase
    }

    /// Whether a login attempt is currently in progress.
    #[must_use]
    pub fn is_in_progress(&self) -> bool {
        matches!(
            self.phase,
            AuthPhase::Starting | AuthPhase::AwaitingApproval(_)
        )
    }

    /// Begin a device-code login. Ignored if already in progress or signed in.
    pub fn start_login(&mut self) {
        if self.is_in_progress() || matches!(self.phase, AuthPhase::SignedIn { .. }) {
            return;
        }
        self.phase = AuthPhase::Starting;
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        let manager = Arc::clone(&self.manager);
        self.task = Some(tokio::spawn(async move {
            run_login(&manager, &tx).await;
        }));
    }

    /// Cancel an in-flight login, returning to the signed-out phase.
    pub fn cancel(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.rx = None;
        if self.is_in_progress() {
            self.phase = AuthPhase::SignedOut;
        }
    }

    /// Sign out, clearing any stored session.
    pub fn sign_out(&mut self, toasts: &mut Toasts) {
        self.cancel();
        match self.manager.sign_out() {
            Ok(()) => {
                self.phase = AuthPhase::SignedOut;
                toasts.info("Signed out.");
            }
            Err(err) => toasts.error(format!("Sign-out failed: {err}")),
        }
    }

    /// Apply any pending background outcomes (non-blocking). Called each tick.
    pub fn drain(&mut self, toasts: &mut Toasts) {
        loop {
            let outcome = match self.rx.as_mut() {
                Some(rx) => match rx.try_recv() {
                    Ok(outcome) => outcome,
                    Err(TryRecvError::Empty) => return,
                    Err(TryRecvError::Disconnected) => {
                        self.rx = None;
                        return;
                    }
                },
                None => return,
            };
            self.apply_outcome(outcome, toasts);
        }
    }

    /// Apply a single outcome to the visible phase.
    fn apply_outcome(&mut self, outcome: AuthOutcome, toasts: &mut Toasts) {
        match outcome {
            AuthOutcome::Pending(pending) => self.phase = AuthPhase::AwaitingApproval(pending),
            AuthOutcome::Completed { expires_at } => {
                self.phase = AuthPhase::SignedIn { expires_at };
                self.rx = None;
                self.task = None;
                toasts.info("Signed in.");
            }
            AuthOutcome::Failed(message) => {
                toasts.error(format!("Login failed: {message}"));
                self.phase = AuthPhase::Failed(message);
                self.rx = None;
                self.task = None;
            }
        }
    }

    /// Await the next background outcome, if a login is in progress.
    #[cfg(test)]
    async fn recv_outcome(&mut self) -> Option<AuthOutcome> {
        match self.rx.as_mut() {
            Some(rx) => rx.recv().await,
            None => None,
        }
    }
}

/// Run the full device-code flow, reporting progress over `tx`.
async fn run_login<T: HttpTransport>(
    manager: &SessionManager<Box<dyn TokenStore>, T>,
    tx: &UnboundedSender<AuthOutcome>,
) {
    let authorization = match manager.client().begin_device_authorization().await {
        Ok(authorization) => authorization,
        Err(err) => {
            let _ = tx.send(AuthOutcome::Failed(err.to_string()));
            return;
        }
    };
    let _ = tx.send(AuthOutcome::Pending(PendingLogin {
        user_code: authorization.user_code.clone(),
        verification_uri: authorization.verification_uri.clone(),
        verification_uri_complete: authorization.verification_uri_complete.clone(),
    }));
    match manager.client().poll_for_token(&authorization).await {
        Ok(tokens) => match manager.complete_login(&tokens, OffsetDateTime::now_utc()) {
            Ok(session) => {
                let _ = tx.send(AuthOutcome::Completed {
                    expires_at: session.expires_at,
                });
            }
            Err(err) => {
                let _ = tx.send(AuthOutcome::Failed(err.to_string()));
            }
        },
        Err(err) => {
            let _ = tx.send(AuthOutcome::Failed(err.to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use tbo_auth::{
        AuthClient, HttpResponse, HttpTransport, InMemoryTokenStore, Result, TokenStore,
    };
    use tbo_core::config::AuthConfig;

    use super::*;

    struct FakeTransport {
        responses: Arc<Mutex<VecDeque<Result<HttpResponse>>>>,
    }

    impl FakeTransport {
        fn new(responses: Vec<Result<HttpResponse>>) -> Self {
            Self {
                responses: Arc::new(Mutex::new(responses.into_iter().collect())),
            }
        }
    }

    impl HttpTransport for FakeTransport {
        async fn post_form(&self, _url: &str, _form: &[(&str, &str)]) -> Result<HttpResponse> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("ran out of responses")
        }
    }

    fn config() -> AuthConfig {
        AuthConfig {
            issuer: "https://auth.example/application/o/test-app".to_owned(),
            client_id: "client-123".to_owned(),
            scopes: "openid offline_access".to_owned(),
        }
    }

    fn controller_with(responses: Vec<Result<HttpResponse>>) -> AuthController<FakeTransport> {
        let store: Box<dyn TokenStore> = Box::new(InMemoryTokenStore::new());
        let client = AuthClient::with_transport(FakeTransport::new(responses), &config());
        let manager = SessionManager::with_client(client, store);
        AuthController::new(Arc::new(manager)).unwrap()
    }

    async fn drive(controller: &mut AuthController<FakeTransport>, toasts: &mut Toasts) {
        while let Some(outcome) = controller.recv_outcome().await {
            controller.apply_outcome(outcome, toasts);
            if matches!(
                controller.phase(),
                AuthPhase::SignedIn { .. } | AuthPhase::Failed(_)
            ) {
                break;
            }
        }
    }

    #[tokio::test]
    async fn starts_signed_out_with_empty_store() {
        let controller = controller_with(vec![]);
        assert!(matches!(controller.phase(), AuthPhase::SignedOut));
    }

    #[tokio::test]
    async fn login_happy_path_signs_in_and_persists() {
        tokio::time::pause();
        let mut controller = controller_with(vec![
            Ok(HttpResponse {
                status: 200,
                body: r#"{"device_code":"dc","user_code":"WXYZ","verification_uri":"https://auth.example/device","expires_in":600,"interval":1}"#
                    .to_owned(),
            }),
            Ok(HttpResponse {
                status: 200,
                body: r#"{"access_token":"at","refresh_token":"rt","expires_in":300}"#.to_owned(),
            }),
        ]);
        let mut toasts = Toasts::default();

        controller.start_login();
        drive(&mut controller, &mut toasts).await;

        assert!(matches!(controller.phase(), AuthPhase::SignedIn { .. }));
        assert!(controller.manager.current_session().unwrap().is_some());
    }

    #[tokio::test]
    async fn login_surfaces_failure() {
        tokio::time::pause();
        let mut controller = controller_with(vec![Ok(HttpResponse {
            status: 400,
            body: r#"{"error":"invalid_client"}"#.to_owned(),
        })]);
        let mut toasts = Toasts::default();

        controller.start_login();
        drive(&mut controller, &mut toasts).await;

        assert!(matches!(controller.phase(), AuthPhase::Failed(_)));
        assert!(controller.manager.current_session().unwrap().is_none());
    }

    #[tokio::test]
    async fn sign_out_clears_session() {
        let mut controller = controller_with(vec![]);
        let mut toasts = Toasts::default();
        controller
            .manager
            .complete_login(
                &serde_json::from_str(
                    r#"{"access_token":"at","refresh_token":"rt","expires_in":300}"#,
                )
                .unwrap(),
                OffsetDateTime::now_utc(),
            )
            .unwrap();

        controller.sign_out(&mut toasts);

        assert!(matches!(controller.phase(), AuthPhase::SignedOut));
        assert!(controller.manager.current_session().unwrap().is_none());
    }
}
