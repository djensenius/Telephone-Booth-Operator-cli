//! HTTP transport abstraction for the operator client.
//!
//! The client is generic over a small [`HttpTransport`] trait so its request
//! construction (paths, query strings, bearer header) and response handling can
//! be unit-tested without a network. Production code uses [`ReqwestTransport`];
//! tests provide an in-memory fake.

use std::future::Future;

use futures::stream::{BoxStream, StreamExt};

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

/// A boxed stream of raw byte chunks from a server-sent events response.
///
/// Chunk boundaries are arbitrary (they may split lines or UTF-8 sequences);
/// the [`SseParser`](crate::sse::SseParser) reassembles whole events.
pub type ByteStream = BoxStream<'static, Result<Vec<u8>>>;

/// Opens long-lived server-sent event streams against the operator API.
///
/// Separate from [`HttpTransport`] so the request/response transport stays
/// simple; production code implements both on [`ReqwestTransport`], while tests
/// can supply canned byte streams.
pub trait SseTransport: HttpTransport {
    /// Issue a streaming `GET` to `path` with `Accept: text/event-stream`,
    /// returning the response body as a stream of byte chunks.
    fn get_sse(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> impl Future<Output = Result<ByteStream>> + Send;
}

/// Performs authenticated mutating requests against the operator API.
///
/// Separate from [`HttpTransport`] (the read transport) so the read-only
/// surface and its test fakes stay simple; production code implements both on
/// [`ReqwestTransport`]. Bodies are pre-serialized JSON strings; `None` sends
/// no body (e.g. action endpoints that take only a path).
pub trait WriteTransport: HttpTransport {
    /// Issue a `POST` to `path` with the given query pairs, optional bearer
    /// token, and an optional JSON request body.
    fn post(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
        json_body: Option<&str>,
    ) -> impl Future<Output = Result<HttpResponse>> + Send;

    /// Issue a `DELETE` to `path` with the given query pairs and optional
    /// bearer token.
    fn delete(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> impl Future<Output = Result<HttpResponse>> + Send;

    /// Issue a raw `PUT` of `body` bytes to an absolute `url` (e.g. an Azure
    /// blob SAS URL), sending `content_type` and the block-blob header the
    /// storage backend requires.
    ///
    /// Unlike [`post`](WriteTransport::post)/[`delete`](WriteTransport::delete)
    /// the `url` is **not** joined to the operator base URL and **no** bearer
    /// token is sent: the short-lived SAS token embedded in the URL is the only
    /// credential. Used by the question-audio upload step.
    fn put_bytes(
        &self,
        url: &str,
        content_type: &str,
        body: Vec<u8>,
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
        send_text(request).await
    }
}

impl SseTransport for ReqwestTransport {
    async fn get_sse(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> Result<ByteStream> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self.client.get(url).header("accept", "text/event-stream");
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
        if !(200..300).contains(&status) {
            let body = response.text().await.unwrap_or_default();
            return Err(match status {
                401 | 403 => OperatorError::Unauthorized(status),
                404 => OperatorError::NotFound,
                _ => OperatorError::Http { status, body },
            });
        }
        let stream = response
            .bytes_stream()
            .map(|chunk| {
                chunk
                    .map(|bytes| bytes.to_vec())
                    .map_err(|err| OperatorError::Transport(err.to_string()))
            })
            .boxed();
        Ok(stream)
    }
}

impl WriteTransport for ReqwestTransport {
    async fn post(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
        json_body: Option<&str>,
    ) -> Result<HttpResponse> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self.client.post(url);
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

    async fn delete(
        &self,
        path: &str,
        query: &[(&str, String)],
        bearer: Option<&str>,
    ) -> Result<HttpResponse> {
        let url = format!("{}{path}", self.base_url.trim_end_matches('/'));
        let mut request = self.client.delete(url);
        if !query.is_empty() {
            request = request.query(query);
        }
        if let Some(token) = bearer {
            request = request.bearer_auth(token);
        }
        send_text(request).await
    }

    async fn put_bytes(
        &self,
        url: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<HttpResponse> {
        let request = self
            .client
            .put(url)
            .header("x-ms-blob-type", "BlockBlob")
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body);
        send_text(request).await
    }
}

/// Send a prepared request and read the response into an [`HttpResponse`].
async fn send_text(request: reqwest::RequestBuilder) -> Result<HttpResponse> {
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
