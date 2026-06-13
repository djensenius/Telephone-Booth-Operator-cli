//! HTTP transport abstraction.
//!
//! The authentication client is generic over a small [`HttpTransport`] trait so
//! the OAuth logic (status handling, error mapping, polling) can be unit-tested
//! without a network or a mock HTTP server. Production code uses
//! [`ReqwestTransport`]; tests provide an in-memory fake.

use std::future::Future;
use std::time::Duration;

use crate::error::{AuthError, Result};

/// Maximum time to establish a TCP/TLS connection before failing. Keeps a login
/// from hanging indefinitely on an unreachable address (e.g. a stalled IPv6
/// route, a proxy, or a silently dropped connection).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Maximum total time for a single request/response. Every auth call (device
/// authorization, token exchange, refresh) is a short request, so a request
/// that exceeds this has stalled and should surface as an error rather than
/// leaving the UI waiting forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// A minimal HTTP response: the status code and the body decoded as UTF-8.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body as text.
    pub body: String,
}

impl HttpResponse {
    /// Whether the status is in the `2xx` range.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// Whether the status is in the `4xx` range.
    #[must_use]
    pub fn is_client_error(&self) -> bool {
        (400..500).contains(&self.status)
    }
}

/// Sends `application/x-www-form-urlencoded` POST requests.
///
/// Implementors must be `Send + Sync` so the authentication client's futures
/// can be sent across threads (e.g. by the async runtime).
pub trait HttpTransport: Send + Sync {
    /// POST the given form fields to `url`, returning the status and body.
    fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
    ) -> impl Future<Output = Result<HttpResponse>> + Send;
}

/// A [`reqwest`]-backed transport using rustls.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Build a transport with a default rustls client.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("tb-operator/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|err| AuthError::Transport(err.to_string()))?;
        Ok(Self { client })
    }

    /// Wrap an existing [`reqwest::Client`].
    #[must_use]
    pub fn from_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl HttpTransport for ReqwestTransport {
    async fn post_form(&self, url: &str, form: &[(&str, &str)]) -> Result<HttpResponse> {
        let response = self
            .client
            .post(url)
            .form(form)
            .send()
            .await
            .map_err(|err| AuthError::Transport(describe(&err)))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|err| AuthError::Transport(describe(&err)))?;
        Ok(HttpResponse { status, body })
    }
}

/// Format an error together with its full [`source`](std::error::Error::source)
/// chain, so a transport failure surfaces its underlying cause (e.g. a connect
/// timeout, DNS failure, or TLS error) instead of reqwest's opaque
/// "error sending request for url (...)".
fn describe<E: std::error::Error>(err: &E) -> String {
    let mut message = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let text = cause.to_string();
        if !message.contains(&text) {
            message.push_str(": ");
            message.push_str(&text);
        }
        source = cause.source();
    }
    message
}
