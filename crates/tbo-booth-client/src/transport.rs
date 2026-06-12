//! HTTP transport abstraction for the booth debug-server client.
//!
//! The client is generic over a small [`BoothTransport`] trait so its request
//! construction (paths, query strings, bearer header) and response handling can
//! be unit-tested without a network. Production code uses
//! [`ReqwestBoothTransport`]; tests provide an in-memory fake.

use std::future::Future;

use crate::error::{BoothError, Result};

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

/// Performs `GET` and `POST` requests against a booth debug server.
///
/// Implementors must be `Send + Sync` so the client's futures can be driven by
/// a multi-threaded runtime. `bearer` is the static debug token to send as
/// `Authorization: Bearer`, or `None` when the booth has no token configured
/// (loopback without auth).
pub trait BoothTransport: Send + Sync {
    /// Issue a `GET` to `path` (relative to the configured base URL) with the
    /// given query pairs and optional bearer token.
    fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> impl Future<Output = Result<HttpResponse>> + Send;

    /// Issue a `POST` to `path` with the given query pairs, optional bearer
    /// token, and an optional JSON request body.
    fn post(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
        json_body: Option<&str>,
    ) -> impl Future<Output = Result<HttpResponse>> + Send;
}

/// A [`reqwest`]-backed transport using rustls.
#[derive(Debug, Clone)]
pub struct ReqwestBoothTransport {
    client: reqwest::Client,
    base_url: String,
}

impl ReqwestBoothTransport {
    /// Build a transport with a default rustls client rooted at `base_url`.
    ///
    /// # Errors
    /// Returns [`BoothError::Transport`] when the HTTP client cannot be
    /// constructed.
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("tb-operator/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|err| BoothError::Transport(err.to_string()))?;
        Ok(Self::from_client(client, base_url))
    }

    /// Wrap an existing [`reqwest::Client`] rooted at `base_url`.
    ///
    /// The custom-TLS (fingerprint-pinned) client lands in a later change; this
    /// constructor lets that client be injected once it exists.
    #[must_use]
    pub fn from_client(client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            client,
            base_url: base_url.into(),
        }
    }

    /// Join `path` onto the configured base URL.
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url.trim_end_matches('/'))
    }
}

impl BoothTransport for ReqwestBoothTransport {
    async fn get(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> Result<HttpResponse> {
        let mut request = self.client.get(self.url(path));
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        send_text(request).await
    }

    async fn post(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
        json_body: Option<&str>,
    ) -> Result<HttpResponse> {
        let mut request = self.client.post(self.url(path));
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        if let Some(body) = json_body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body.to_owned());
        }
        send_text(request).await
    }
}

/// Send a prepared request and read the response into an [`HttpResponse`].
async fn send_text(request: reqwest::RequestBuilder) -> Result<HttpResponse> {
    let response = request
        .send()
        .await
        .map_err(|err| BoothError::Transport(err.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|err| BoothError::Transport(err.to_string()))?;
    Ok(HttpResponse { status, body })
}
