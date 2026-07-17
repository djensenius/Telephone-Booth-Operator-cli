//! Bearer-authenticated WebSocket client for live operator status updates.
//!
//! The operator API exposes `/v1/ws/status` as a WebSocket that emits JSON
//! [`WsEnvelope`](tbo_core::domain::WsEnvelope) text frames. This module handles
//! URL derivation, the authenticated handshake, TLS via the workspace rustls
//! stack, and decoding the incoming frames into a stream.

use std::sync::Arc;

use futures::stream::{BoxStream, Stream, StreamExt};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, RootCertStore};
use tbo_core::domain::WsEnvelope;
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, HeaderValue};
use tokio_tungstenite::{WebSocketStream, client_async};
use tracing::warn;

use crate::error::{OperatorError, Result};

/// Path of the operator API's live status WebSocket.
const STATUS_PATH: &str = "/v1/ws/status";

/// A boxed stream of decoded live status WebSocket envelopes.
///
/// Decode errors are logged and skipped. The stream ends when the server closes
/// the socket; transport errors are yielded once so callers can record the
/// failure and reconnect with their own backoff.
pub type StatusEnvelopeStream = BoxStream<'static, Result<WsEnvelope>>;

/// Connect to the operator live-status WebSocket and decode envelope frames.
///
/// `base_url` is the operator API HTTP base URL; its scheme is rewritten to
/// `ws`/`wss` and `/v1/ws/status` is appended. `token` is sent as
/// `Authorization: Bearer <token>` during the WebSocket handshake.
///
/// # Errors
/// Returns [`OperatorError::InvalidRequest`] for malformed URLs or header
/// values, and [`OperatorError::Transport`] for TCP, TLS, or handshake errors.
pub async fn connect_status(base_url: &str, token: &str) -> Result<StatusEnvelopeStream> {
    let ws_url = status_ws_url(base_url)?;
    let uri: Uri = ws_url.parse().map_err(|err| {
        OperatorError::InvalidRequest(format!("invalid status WebSocket URL: {err}"))
    })?;
    let secure = uri.scheme_str() == Some("wss");
    let host = uri
        .host()
        .ok_or_else(|| OperatorError::InvalidRequest("operator URL has no host".to_owned()))?
        .to_owned();
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });

    let mut request = ws_url
        .into_client_request()
        .map_err(|err| OperatorError::Transport(err.to_string()))?;
    let value = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|err| OperatorError::InvalidRequest(format!("invalid bearer token: {err}")))?;
    request.headers_mut().insert(AUTHORIZATION, value);

    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|err| OperatorError::Transport(err.to_string()))?;

    if secure {
        let server_name = ServerName::try_from(host)
            .map_err(|err| OperatorError::InvalidRequest(format!("invalid server name: {err}")))?;
        let tls = TlsConnector::from(Arc::new(public_tls_config()?))
            .connect(server_name, tcp)
            .await
            .map_err(|err| OperatorError::Transport(err.to_string()))?;
        let (ws, _response) = client_async(request, tls)
            .await
            .map_err(|err| OperatorError::Transport(err.to_string()))?;
        Ok(decode_messages(ws).boxed())
    } else {
        let (ws, _response) = client_async(request, tcp)
            .await
            .map_err(|err| OperatorError::Transport(err.to_string()))?;
        Ok(decode_messages(ws).boxed())
    }
}

/// Build a rustls client config rooted in the public WebPKI trust anchors.
fn public_tls_config() -> Result<ClientConfig> {
    let roots = webpki_roots::TLS_SERVER_ROOTS
        .iter()
        .cloned()
        .collect::<RootCertStore>();
    ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|err| OperatorError::Transport(err.to_string()))
        .map(|builder| builder.with_root_certificates(roots).with_no_client_auth())
}

/// Adapt WebSocket frames into decoded envelopes.
fn decode_messages<S>(messages: WebSocketStream<S>) -> impl Stream<Item = Result<WsEnvelope>> + Send
where
    WebSocketStream<S>: Stream<Item = std::result::Result<Message, WsError>> + Send + 'static,
{
    messages.filter_map(|message| async move {
        match message {
            Ok(Message::Text(text)) => parse_envelope(text.as_str()),
            Ok(Message::Close(_)) => None,
            Ok(_) => None,
            Err(err) => Some(Err(OperatorError::Transport(err.to_string()))),
        }
    })
}

/// Decode a single text frame, logging and skipping malformed JSON.
fn parse_envelope(text: &str) -> Option<Result<WsEnvelope>> {
    match serde_json::from_str(text) {
        Ok(envelope) => Some(Ok(envelope)),
        Err(err) => {
            warn!(error = %err, "skipping malformed status WebSocket frame");
            None
        }
    }
}

/// Rewrite an operator API base URL into its live-status WebSocket URL.
pub fn status_ws_url(base_url: &str) -> Result<String> {
    let base = base_url.trim().trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("wss://") || base.starts_with("ws://") {
        base.to_owned()
    } else {
        return Err(OperatorError::InvalidRequest(format!(
            "operator URL must start with http(s):// or ws(s)://, got {base_url:?}"
        )));
    };
    Ok(format!("{ws_base}{STATUS_PATH}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use tbo_core::domain::BoothState;

    #[test]
    fn ws_url_rewrites_scheme_and_appends_path() {
        assert_eq!(
            status_ws_url("http://127.0.0.1:8080").unwrap(),
            "ws://127.0.0.1:8080/v1/ws/status"
        );
        assert_eq!(
            status_ws_url("https://api.example.com/").unwrap(),
            "wss://api.example.com/v1/ws/status"
        );
        assert_eq!(
            status_ws_url("wss://api.example.com/base").unwrap(),
            "wss://api.example.com/base/v1/ws/status"
        );
    }

    #[test]
    fn ws_url_rejects_unknown_scheme() {
        assert!(status_ws_url("ftp://api.example.com").is_err());
    }

    #[test]
    fn parse_envelope_decodes_status_frame_and_skips_bad_json() {
        let envelope = parse_envelope(
            r#"{"kind":"status","status":{"state":"idle","updatedAt":"2026-01-01T00:00:00Z"}}"#,
        )
        .unwrap()
        .unwrap();

        assert!(matches!(
            envelope,
            WsEnvelope::Status { status } if status.state == BoothState::Idle
        ));
        assert!(parse_envelope("not json").is_none());
    }
}
