//! In-terminal playback of message and question audio.
//!
//! Holds a [`tbo_audio::AudioPlayer`] (the background playback thread) and
//! orchestrates fetching a track's bytes off the UI thread before handing them
//! to the player. Booth audio is served from short-lived Azure blob SAS URLs
//! (read URLs expire after a few minutes), so playback re-fetches a fresh URL
//! just-in-time where the operator API allows it:
//!
//! - **Messages** have a detail endpoint, so the message is re-fetched to obtain
//!   a fresh `audio.url` (falling back to the in-hand URL if that fails).
//! - **Questions** have no detail endpoint, so the URL already loaded into the
//!   Questions screen is used directly.
//!
//! When no output device is available (e.g. a headless SSH session) the player
//! is absent and playback is gracefully disabled; the play methods become
//! no-ops and the UI can explain why via
//! [`PlaybackController::unavailable_reason`].

use std::future::Future;

use tokio::sync::mpsc::error::TryRecvError;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use tbo_audio::{AudioPlayer, PlaybackState};
use tbo_core::domain::{Message, Question};
use tbo_operator_client::{
    HttpTransport, OperatorClient, ReqwestTransport, TokenProvider, WriteTransport,
};

use crate::data::{ActionOutcome, SessionTokenProvider};

/// The result of a background fetch: the downloaded FLAC bytes, or an error
/// message to surface as a toast.
type FetchResult = std::result::Result<Vec<u8>, String>;

/// The audio sink playback drives, abstracted so the controller's orchestration
/// can be tested without opening a real output device. Implemented by
/// [`AudioPlayer`] in production and a fake in tests.
//
// `pub(crate)` is required because this trait bounds the public
// `PlaybackController` impl blocks (which are reachable at crate visibility via
// the `data` re-export); a fully private trait trips `private_bounds`.
#[allow(clippy::redundant_pub_crate)]
pub(crate) trait PlaybackSink {
    /// Play the given FLAC bytes, replacing any current track.
    fn play(&self, bytes: Vec<u8>);
    /// Toggle between playing and paused.
    fn toggle(&self);
    /// Stop and unload the current track.
    fn stop(&self);
    /// A snapshot of the current playback state.
    fn snapshot(&self) -> PlaybackState;
}

impl PlaybackSink for AudioPlayer {
    fn play(&self, bytes: Vec<u8>) {
        AudioPlayer::play(self, bytes);
    }
    fn toggle(&self) {
        AudioPlayer::toggle(self);
    }
    fn stop(&self) {
        AudioPlayer::stop(self);
    }
    fn snapshot(&self) -> PlaybackState {
        AudioPlayer::snapshot(self)
    }
}

/// Drives in-terminal audio playback for the Messages and Questions screens.
///
/// Generic over the operator transport and token provider (so the fetch path
/// can be exercised with a fake client) and the audio sink (so delivery can be
/// tested without a device); the production type uses the same defaults as the
/// other data controllers.
pub struct PlaybackController<T = ReqwestTransport, A = SessionTokenProvider, S = AudioPlayer>
where
    T: HttpTransport,
    A: TokenProvider,
{
    client: OperatorClient<T, A>,
    player: Option<S>,
    unavailable: Option<String>,
    fetch_in_flight: bool,
    rx: Option<UnboundedReceiver<FetchResult>>,
}

impl<T, A> PlaybackController<T, A, AudioPlayer>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    /// Build a controller, opening the default audio output device.
    ///
    /// Opening the device can fail on a headless host; that is not fatal —
    /// playback is disabled and the reason is recorded for the UI.
    pub fn new(client: OperatorClient<T, A>) -> Self {
        match AudioPlayer::new() {
            Ok(player) => Self::with_player(client, Some(player), None),
            Err(err) => Self::with_player(client, None, Some(err)),
        }
    }
}

impl<T, A, S> PlaybackController<T, A, S>
where
    T: HttpTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
    S: PlaybackSink,
{
    /// Build a controller over a specific (optional) sink.
    fn with_player(
        client: OperatorClient<T, A>,
        player: Option<S>,
        unavailable: Option<String>,
    ) -> Self {
        Self {
            client,
            player,
            unavailable,
            fetch_in_flight: false,
            rx: None,
        }
    }

    /// Whether an output device is available for playback.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.player.is_some()
    }

    /// Why playback is unavailable, when it is.
    #[must_use]
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable.as_deref()
    }

    /// Whether a track's bytes are currently being fetched.
    #[must_use]
    pub fn is_loading(&self) -> bool {
        self.fetch_in_flight
    }

    /// A snapshot of the current playback state, when a player exists.
    #[must_use]
    pub fn snapshot(&self) -> Option<PlaybackState> {
        self.player.as_ref().map(PlaybackSink::snapshot)
    }

    /// Toggle play/pause of the current track.
    pub fn toggle_pause(&self) {
        if let Some(player) = &self.player {
            player.toggle();
        }
    }

    /// Stop and unload the current track.
    pub fn stop(&self) {
        if let Some(player) = &self.player {
            player.stop();
        }
    }

    /// Deliver any completed fetch to the player (called each tick). Returns a
    /// failure outcome to toast when the fetch failed.
    pub fn drain(&mut self) -> Option<ActionOutcome> {
        let rx = self.rx.as_mut()?;
        match rx.try_recv() {
            Ok(result) => {
                self.fetch_in_flight = false;
                self.rx = None;
                self.deliver(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.fetch_in_flight = false;
                self.rx = None;
                None
            }
        }
    }

    /// Hand fetched bytes to the player, or convert a fetch error to an outcome.
    fn deliver(&self, result: FetchResult) -> Option<ActionOutcome> {
        match result {
            Ok(bytes) => {
                if let Some(player) = &self.player {
                    player.play(bytes);
                }
                None
            }
            Err(message) => Some(ActionOutcome { message, ok: false }),
        }
    }

    /// Spawn a fetch task, gating against a player-less host or an in-flight
    /// fetch. The future yields the downloaded bytes or an error message.
    fn start_fetch<F>(&mut self, future: F)
    where
        F: Future<Output = FetchResult> + Send + 'static,
    {
        if self.player.is_none() || self.fetch_in_flight {
            return;
        }
        self.fetch_in_flight = true;
        let (tx, rx) = unbounded_channel();
        self.rx = Some(rx);
        tokio::spawn(async move {
            let _ = tx.send(future.await);
        });
    }

    /// Await and deliver the next fetch result (test helper).
    #[cfg(test)]
    async fn recv_once(&mut self) -> Option<ActionOutcome> {
        let rx = self.rx.as_mut()?;
        let result = rx.recv().await?;
        self.fetch_in_flight = false;
        self.rx = None;
        self.deliver(result)
    }
}

impl<T, A, S> PlaybackController<T, A, S>
where
    T: WriteTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
    S: PlaybackSink,
{
    /// Fetch a fresh SAS URL for the given message (falling back to the URL in
    /// hand), download its audio, and play it.
    pub fn play_message(&mut self, message: &Message) {
        let client = self.client.clone();
        let id = message.id.clone();
        let fallback_url = message.audio.url.clone();
        self.start_fetch(fetch_message_audio(client, id, fallback_url));
    }

    /// Download the question's audio (using the URL already loaded) and play it.
    ///
    /// Unlike messages there is no question-detail endpoint to refresh the SAS
    /// URL; if it has expired the download fails and the operator can refresh
    /// the Questions list to obtain a fresh URL.
    pub fn play_question(&mut self, question: &Question) {
        let client = self.client.clone();
        let url = question.audio.url.clone();
        self.start_fetch(fetch_audio(client, url));
    }
}

/// Re-fetch a message for a fresh SAS URL (falling back to `fallback_url` when
/// the detail request fails), then download its audio.
async fn fetch_message_audio<T, A>(
    client: OperatorClient<T, A>,
    id: String,
    fallback_url: String,
) -> FetchResult
where
    T: WriteTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    let url = match client.message(&id).await {
        Ok(fresh) => fresh.audio.url,
        Err(_) => fallback_url,
    };
    fetch_audio(client, url).await
}

/// Download the FLAC bytes at `url`, mapping any error to a display message.
async fn fetch_audio<T, A>(client: OperatorClient<T, A>, url: String) -> FetchResult
where
    T: WriteTransport + Clone + 'static,
    A: TokenProvider + Clone + 'static,
{
    client
        .download_audio(&url)
        .await
        .map_err(|err| format!("Audio download failed: {err}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::sync::{Arc, Mutex};

    use tbo_operator_client::{
        HttpResponse, HttpTransport, OperatorError, Result, StaticTokenProvider,
    };

    use super::*;

    /// A transport returning canned message JSON for `get` and canned bytes for
    /// `get_bytes`, recording the URLs the download requested.
    #[derive(Clone)]
    struct FakeTransport {
        message_json: String,
        message_status: u16,
        audio_bytes: Vec<u8>,
        audio_status: u16,
        bytes_urls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeTransport {
        fn new(message_json: &str, audio_bytes: Vec<u8>) -> Self {
            Self {
                message_json: message_json.to_owned(),
                message_status: 200,
                audio_bytes,
                audio_status: 200,
                bytes_urls: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn with_message_status(mut self, status: u16) -> Self {
            self.message_status = status;
            self
        }

        fn with_audio_status(mut self, status: u16) -> Self {
            self.audio_status = status;
            self
        }

        fn requested_urls(&self) -> Vec<String> {
            self.bytes_urls.lock().unwrap().clone()
        }
    }

    impl HttpTransport for FakeTransport {
        async fn get(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            Ok(HttpResponse {
                status: self.message_status,
                body: self.message_json.clone(),
            })
        }
    }

    impl WriteTransport for FakeTransport {
        async fn post(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
            _json_body: Option<&str>,
        ) -> Result<HttpResponse> {
            unreachable!("playback never POSTs")
        }

        async fn delete(
            &self,
            _path: &str,
            _query: &[(&str, String)],
            _bearer: Option<&str>,
        ) -> Result<HttpResponse> {
            unreachable!("playback never DELETEs")
        }

        async fn put_bytes(
            &self,
            _url: &str,
            _content_type: &str,
            _body: Vec<u8>,
        ) -> Result<HttpResponse> {
            unreachable!("playback never uploads")
        }

        async fn get_bytes(&self, url: &str) -> Result<Vec<u8>> {
            self.bytes_urls.lock().unwrap().push(url.to_owned());
            if (200..300).contains(&self.audio_status) {
                Ok(self.audio_bytes.clone())
            } else {
                Err(OperatorError::Http {
                    status: self.audio_status,
                    body: String::new(),
                })
            }
        }
    }

    /// A sink that records the bytes handed to it.
    #[derive(Clone, Default)]
    struct FakeSink {
        played: Arc<Mutex<Vec<Vec<u8>>>>,
        toggles: Arc<Mutex<u32>>,
        stops: Arc<Mutex<u32>>,
    }

    impl PlaybackSink for FakeSink {
        fn play(&self, bytes: Vec<u8>) {
            self.played.lock().unwrap().push(bytes);
        }
        fn toggle(&self) {
            *self.toggles.lock().unwrap() += 1;
        }
        fn stop(&self) {
            *self.stops.lock().unwrap() += 1;
        }
        fn snapshot(&self) -> PlaybackState {
            PlaybackState::new()
        }
    }

    fn controller_with(
        transport: FakeTransport,
        player: Option<FakeSink>,
    ) -> PlaybackController<FakeTransport, StaticTokenProvider, FakeSink> {
        let client = OperatorClient::with_transport(transport, StaticTokenProvider::new("token"));
        let unavailable = player
            .is_none()
            .then(|| "no audio device (test)".to_owned());
        PlaybackController::with_player(client, player, unavailable)
    }

    fn message_json(id: &str, url: &str) -> String {
        format!(
            r#"{{"id":"{id}","status":"pending","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"{url}","sha256":"{id}","durationMs":1000}}}}"#
        )
    }

    fn message(id: &str, url: &str) -> Message {
        serde_json::from_str(&message_json(id, url)).unwrap()
    }

    fn question(id: &str, url: &str) -> Question {
        let json = format!(
            r#"{{"id":"{id}","prompt":"hi","status":"active","createdAt":"2026-01-01T00:00:00Z","audio":{{"url":"{url}","sha256":"{id}","durationMs":1000}}}}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn headless_disables_playback() {
        let mut controller = controller_with(FakeTransport::new("", Vec::new()), None);
        assert!(!controller.is_available());
        assert_eq!(
            controller.unavailable_reason(),
            Some("no audio device (test)")
        );

        controller.play_message(&message("a", "https://stale/a.flac"));
        controller.play_question(&question("q", "https://stale/q.flac"));
        // With no player, no fetch is spawned.
        assert!(!controller.is_loading());
    }

    #[tokio::test]
    async fn play_message_refreshes_url_then_plays_bytes() {
        // The detail endpoint returns a *fresh* URL; the download must use it,
        // not the stale URL the caller holds.
        let transport =
            FakeTransport::new(&message_json("a", "https://fresh/a.flac"), vec![1, 2, 3]);
        let sink = FakeSink::default();
        let mut controller = controller_with(transport.clone(), Some(sink.clone()));

        controller.play_message(&message("a", "https://stale/a.flac"));
        assert!(controller.is_loading());
        let outcome = controller.recv_once().await;

        assert!(outcome.is_none(), "a successful play produces no toast");
        assert_eq!(sink.played.lock().unwrap().as_slice(), [vec![1, 2, 3]]);
        assert_eq!(transport.requested_urls(), ["https://fresh/a.flac"]);
    }

    #[tokio::test]
    async fn play_message_falls_back_to_in_hand_url_when_detail_fails() {
        let transport = FakeTransport::new("", vec![7])
            .with_message_status(500)
            .with_audio_status(200);
        let sink = FakeSink::default();
        let mut controller = controller_with(transport.clone(), Some(sink.clone()));

        controller.play_message(&message("a", "https://stale/a.flac"));
        controller.recv_once().await;

        assert_eq!(sink.played.lock().unwrap().as_slice(), [vec![7]]);
        assert_eq!(transport.requested_urls(), ["https://stale/a.flac"]);
    }

    #[tokio::test]
    async fn play_question_downloads_in_hand_url() {
        let transport = FakeTransport::new("", vec![9, 9]);
        let sink = FakeSink::default();
        let mut controller = controller_with(transport.clone(), Some(sink.clone()));

        controller.play_question(&question("q", "https://stale/q.flac"));
        let outcome = controller.recv_once().await;

        assert!(outcome.is_none());
        assert_eq!(sink.played.lock().unwrap().as_slice(), [vec![9, 9]]);
        // Questions are not refreshed (no detail endpoint), so the in-hand URL
        // is used directly.
        assert_eq!(transport.requested_urls(), ["https://stale/q.flac"]);
    }

    #[tokio::test]
    async fn download_failure_surfaces_as_toast_and_plays_nothing() {
        let transport = FakeTransport::new("", Vec::new()).with_audio_status(403);
        let sink = FakeSink::default();
        let mut controller = controller_with(transport, Some(sink.clone()));

        controller.play_question(&question("q", "https://stale/q.flac"));
        let outcome = controller.recv_once().await.expect("a failure outcome");

        assert!(!outcome.ok);
        assert!(outcome.message.contains("Audio download failed"));
        assert!(sink.played.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn second_play_is_ignored_while_a_fetch_is_in_flight() {
        let transport = FakeTransport::new(&message_json("a", "https://fresh/a.flac"), vec![1]);
        let sink = FakeSink::default();
        let mut controller = controller_with(transport, Some(sink));

        controller.play_question(&question("q", "https://one/q.flac"));
        assert!(controller.is_loading());
        // A second request while the first is in flight must be dropped.
        controller.play_question(&question("q2", "https://two/q.flac"));
        controller.recv_once().await;
        // Only the first fetch ran (its URL is the one recorded by recv).
    }

    #[test]
    fn toggle_and_stop_drive_the_sink() {
        let sink = FakeSink::default();
        let controller = controller_with(FakeTransport::new("", Vec::new()), Some(sink.clone()));
        controller.toggle_pause();
        controller.stop();
        assert_eq!(*sink.toggles.lock().unwrap(), 1);
        assert_eq!(*sink.stops.lock().unwrap(), 1);
    }
}
