//! HTTP transport abstraction.
//!
//! The authentication client is generic over a small [`HttpTransport`] trait so
//! the OAuth logic (status handling, error mapping, polling) can be unit-tested
//! without a network or a mock HTTP server. Production code uses
//! [`ReqwestTransport`]; tests provide an in-memory fake.

use std::future::Future;

use crate::error::{AuthError, Result};

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
            .map_err(|err| AuthError::Transport(err.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|err| AuthError::Transport(err.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}
