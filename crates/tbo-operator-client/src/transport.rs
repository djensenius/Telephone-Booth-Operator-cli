//! HTTP transport abstraction for the operator client.
//!
//! The client is generic over a small [`HttpTransport`] trait so its request
//! construction (paths, query strings, bearer header) and response handling can
//! be unit-tested without a network. Production code uses [`ReqwestTransport`];
//! tests provide an in-memory fake.

use std::future::Future;

use crate::error::{OperatorError, Result};

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
}

/// Performs authenticated `GET` requests against the operator API.
///
/// Implementors must be `Send + Sync` so the client's futures can be driven by
/// a multi-threaded runtime. `query` carries already-decoded key/value pairs
/// (repeated keys encode array parameters); `bearer` is the access token to
/// send as `Authorization: Bearer`, or `None` for anonymous endpoints.
pub trait HttpTransport: Send + Sync {
    /// Issue a `GET` to `path` (relative to the configured base URL) with the
    /// given query pairs and optional bearer token.
    fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> impl Future<Output = Result<HttpResponse>> + Send;
}

/// A [`reqwest`]-backed transport using rustls.
#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestTransport {
    /// Build a transport with a default rustls client rooted at `base_url`.
    ///
    /// # Errors
    /// Returns [`OperatorError::Transport`] when the HTTP client cannot be
    /// constructed.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("tb-operator/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| OperatorError::Transport(err.to_string()))?;
        Ok(Self::from_client(client, base_url))
    }

    /// Wrap an existing [`reqwest::Client`] rooted at `base_url`.
    #[must_use]
    pub fn from_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into(),
        }
    }
}

impl HttpTransport for ReqwestTransport {
    async fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> Result<HttpResponse> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self.client.get(url);
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .map_err(|err| OperatorError::Transport(err.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .text()
            .await
            .map_err(|err| OperatorError::Transport(err.to_string()))?;
        Ok(HttpResponse { status, body })
    }
}
