//! Live telemetry stream from a booth's debug-server WebSocket.
//!
//! The booth debug server exposes `/v1/ws/telemetry`: after the handshake the
//! client may send a `{ "replay_from": <id> }` frame to request buffered
//! records, then the server streams JSON [`TelemetryRecord`] text frames as
//! events occur. Authentication reuses the static debug token as a bearer
//! header.
//!
//! [`connect_telemetry`] dials the socket — plain `ws://` for the loopback
//! endpoint, or `wss://` with [TLS fingerprint pinning](crate::tls) for the LAN
//! endpoint — and yields a [`TelemetryStream`] of decoded records. The decoding
//! is factored into a small internal adapter so it can be unit-tested without a
//! network.

use std::sync::Arc;

use futures::SinkExt;
use futures::stream::{BoxStream, Stream, StreamExt};
use rustls::pki_types::ServerName;
use serde::Serialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::tungstenite::http::header::{AUTHORIZATION, HeaderValue};
use tokio_tungstenite::{WebSocketStream, client_async};

use crate::error::{BoothError, Result};
use crate::model::TelemetryRecord;
use crate::tls::pinned_tls_config;

/// Path of the booth debug server's telemetry WebSocket.
const TELEMETRY_PATH: &str = "/v1/ws/telemetry";

/// A boxed stream of live [`TelemetryRecord`]s from the booth telemetry socket.
///
/// The stream ends (`None`) when the server closes the connection; a transport
/// or decode failure is yielded as an `Err` item.
pub type TelemetryStream = BoxStream<'static, Result<TelemetryRecord>>;

/// Asks the server to replay buffered records with an id `>= replay_from`
/// before streaming live frames.
#[derive(Debug, Serialize)]
struct ReplayRequest {
    /// Lowest record id to replay.
    replay_from: u64,
}

/// Connect to a booth's telemetry WebSocket and stream decoded records.
///
/// `base_url` is the booth's debug HTTP base (`http://…:8080` loopback or
/// `https://…:8443` LAN, `ws`/`wss` are also accepted); the scheme is rewritten
/// to `ws`/`wss` and the telemetry path appended. `token` is sent as
/// `Authorization: Bearer`. For a `wss` endpoint the self-signed certificate is
/// pinned to `pinned_sha256`, which is **required** for LAN TLS. When
/// `replay_from` is `Some(n)`, the server first replays buffered records from
/// id `n` before live frames.
///
/// # Errors
/// Returns [`BoothError::InvalidRequest`] for a malformed URL, an invalid
/// token, or a `wss` endpoint without a (valid) fingerprint, or
/// [`BoothError::Transport`] when the TCP/TLS connection or the WebSocket
/// handshake fails.
pub async fn connect_telemetry(
    base_url: &str,
    token: Option<&str>,
    pinned_sha256: Option<&str>,
    replay_from: Option<u64>,
) -> Result<TelemetryStream> {
    let ws_url = telemetry_ws_url(base_url)?;
    let uri: Uri = ws_url
        .parse()
        .map_err(|err| BoothError::InvalidRequest(format!("invalid booth URL: {err}")))?;
    let secure = uri.scheme_str() == Some("wss");
    let host = uri
        .host()
        .ok_or_else(|| BoothError::InvalidRequest("booth URL has no host".to_owned()))?
        .to_owned();
    let port = uri.port_u16().unwrap_or(if secure { 443 } else { 80 });

    // Build the pinned TLS config up front so a misconfigured `wss` endpoint
    // fails before any network I/O.
    let tls_config = if secure {
        let fingerprint = pinned_sha256
            .map(str::trim)
            .filter(|fingerprint| !fingerprint.is_empty())
            .ok_or_else(|| {
                BoothError::InvalidRequest(
                    "booth LAN TLS (wss) requires a pinned certificate fingerprint".to_owned(),
                )
            })?;
        Some(Arc::new(pinned_tls_config(fingerprint)?))
    } else {
        None
    };

    let mut request = ws_url
        .into_client_request()
        .map_err(|err| BoothError::Transport(err.to_string()))?;
    if let Some(token) = token {
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|err| BoothError::InvalidRequest(format!("invalid token: {err}")))?;
        request.headers_mut().insert(AUTHORIZATION, value);
    }

    let tcp = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|err| BoothError::Transport(err.to_string()))?;

    if let Some(config) = tls_config {
        let server_name = ServerName::try_from(host)
            .map_err(|err| BoothError::InvalidRequest(format!("invalid server name: {err}")))?;
        let tls = TlsConnector::from(config)
            .connect(server_name, tcp)
            .await
            .map_err(|err| BoothError::Transport(err.to_string()))?;
        let (mut ws, _response) = client_async(request, tls)
            .await
            .map_err(|err| BoothError::Transport(err.to_string()))?;
        send_replay(&mut ws, replay_from).await?;
        Ok(decode_messages(ws).boxed())
    } else {
        let (mut ws, _response) = client_async(request, tcp)
            .await
            .map_err(|err| BoothError::Transport(err.to_string()))?;
        send_replay(&mut ws, replay_from).await?;
        Ok(decode_messages(ws).boxed())
    }
}

/// Send the optional replay request on a freshly-opened socket.
async fn send_replay<S>(ws: &mut WebSocketStream<S>, replay_from: Option<u64>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(replay_from) = replay_from {
        let body = serde_json::to_string(&ReplayRequest { replay_from })
            .map_err(|err| BoothError::InvalidRequest(err.to_string()))?;
        ws.send(Message::Text(body.into()))
            .await
            .map_err(|err| BoothError::Transport(err.to_string()))?;
    }
    Ok(())
}

/// Adapt a stream of raw WebSocket messages into decoded telemetry records.
///
/// Text and binary frames are parsed as [`TelemetryRecord`]; ping/pong/close
/// control frames are skipped; a WebSocket-level error is surfaced as a
/// transport error.
fn decode_messages<S>(messages: S) -> impl Stream<Item = Result<TelemetryRecord>> + Send
where
    S: Stream<Item = std::result::Result<Message, WsError>> + Send + 'static,
{
    messages.filter_map(|message| async move {
        match message {
            Ok(Message::Text(text)) => Some(parse_record(text.as_str())),
            Ok(Message::Binary(bytes)) => Some(parse_record_bytes(&bytes)),
            Ok(_) => None,
            Err(err) => Some(Err(BoothError::Transport(err.to_string()))),
        }
    })
}

/// Decode a single telemetry frame from text.
fn parse_record(text: &str) -> Result<TelemetryRecord> {
    serde_json::from_str(text).map_err(|err| BoothError::Decode(err.to_string()))
}

/// Decode a single telemetry frame from raw bytes.
fn parse_record_bytes(bytes: &[u8]) -> Result<TelemetryRecord> {
    serde_json::from_slice(bytes).map_err(|err| BoothError::Decode(err.to_string()))
}

/// Rewrite a booth debug base URL into its telemetry WebSocket URL.
fn telemetry_ws_url(base_url: &str) -> Result<String> {
    let base = base_url.trim().trim_end_matches('/');
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if base.starts_with("wss://") || base.starts_with("ws://") {
        base.to_owned()
    } else {
        return Err(BoothError::InvalidRequest(format!(
            "booth URL must start with http(s):// or ws(s)://, got {base_url:?}"
        )));
    };
    Ok(format!("{ws_base}{TELEMETRY_PATH}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::model::{GpioEdge, TelemetryEvent};
    use tokio_tungstenite::tungstenite::protocol::frame::Utf8Bytes;

    const GPIO_FRAME: &str = r#"{
        "id": 7,
        "ts": { "secs_since_epoch": 1700000000, "nanos_since_epoch": 0 },
        "kind": "gpio_edge",
        "role": "hook",
        "level": true,
        "at_monotonic_ns": 12345
    }"#;

    #[test]
    fn ws_url_rewrites_scheme_and_appends_path() {
        assert_eq!(
            telemetry_ws_url("http://127.0.0.1:8080").unwrap(),
            "ws://127.0.0.1:8080/v1/ws/telemetry"
        );
        assert_eq!(
            telemetry_ws_url("https://booth.ts.net:8443/").unwrap(),
            "wss://booth.ts.net:8443/v1/ws/telemetry"
        );
        assert_eq!(
            telemetry_ws_url("ws://localhost:9000").unwrap(),
            "ws://localhost:9000/v1/ws/telemetry"
        );
    }

    #[test]
    fn ws_url_rejects_unknown_scheme() {
        assert!(telemetry_ws_url("ftp://booth").is_err());
    }

    #[test]
    fn parse_record_decodes_and_reports_errors() {
        let record = parse_record(GPIO_FRAME).unwrap();
        assert_eq!(record.id, 7);
        assert!(matches!(
            parse_record("not json"),
            Err(BoothError::Decode(_))
        ));
    }

    #[tokio::test]
    async fn decodes_text_frames_and_skips_control_frames() {
        let messages = futures::stream::iter(vec![
            Ok(Message::Ping(Vec::new().into())),
            Ok(Message::Text(Utf8Bytes::from(GPIO_FRAME))),
            Ok(Message::Pong(Vec::new().into())),
            Ok(Message::Close(None)),
        ]);
        let mut stream = decode_messages(messages).boxed();

        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(first.id, 7);
        assert_eq!(
            first.event,
            TelemetryEvent::GpioEdge(GpioEdge {
                role: "hook".to_owned(),
                level: true,
                at_monotonic_ns: 12345,
            })
        );
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn surfaces_websocket_errors() {
        let messages = futures::stream::iter(vec![Err(WsError::ConnectionClosed)]);
        let mut stream = decode_messages(messages).boxed();
        assert!(matches!(
            stream.next().await,
            Some(Err(BoothError::Transport(_)))
        ));
    }

    #[tokio::test]
    async fn wss_without_fingerprint_is_rejected_before_connecting() {
        let result = connect_telemetry("https://booth.ts.net:8443", Some("tok"), None, None).await;
        assert!(matches!(result, Err(BoothError::InvalidRequest(_))));
    }

    #[tokio::test]
    async fn wss_with_invalid_fingerprint_is_rejected_before_connecting() {
        let result =
            connect_telemetry("https://booth.ts.net:8443", None, Some("not-hex"), None).await;
        assert!(matches!(result, Err(BoothError::InvalidRequest(_))));
    }
}
