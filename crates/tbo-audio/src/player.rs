//! [`AudioPlayer`]: a handle to a background thread that owns the (thread-bound)
//! audio output and plays one track at a time.
//!
//! The UI thread sends commands over a channel and reads a cloned
//! [`PlaybackState`] snapshot for rendering; the background thread translates
//! commands into [`AudioBackend`] calls and polls for completion. The
//! command-handling and progress-polling logic is factored into free functions
//! so it can be tested with a fake backend, without a real device or thread.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::backend::{AudioBackend, RodioBackend};
use crate::state::{PlaybackState, PlaybackStatus};

/// How often the playback thread polls for end-of-track / position updates
/// while idle between commands.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A command sent to the playback thread.
enum Command {
    /// Decode and play the given FLAC bytes, replacing any current track.
    Play(Vec<u8>),
    /// Pause the current track.
    Pause,
    /// Resume the current track.
    Resume,
    /// Stop and unload the current track.
    Stop,
    /// Tear the thread down.
    Quit,
}

/// A handle to the background audio playback thread.
///
/// Cloning is intentionally not supported: the player owns its thread and tears
/// it down on drop.
pub struct AudioPlayer {
    tx: Sender<Command>,
    shared: Arc<Mutex<PlaybackState>>,
    handle: Option<JoinHandle<()>>,
}

impl AudioPlayer {
    /// Open the default output device and start the playback thread.
    ///
    /// # Errors
    /// Returns a message when no audio output device is available.
    pub fn new() -> Result<Self, String> {
        let backend = RodioBackend::open()?;
        Ok(Self::spawn(backend))
    }

    /// Start the playback thread over an arbitrary backend.
    fn spawn<B: AudioBackend + Send + 'static>(backend: B) -> Self {
        let (tx, rx) = channel();
        let shared = Arc::new(Mutex::new(PlaybackState::new()));
        let thread_shared = Arc::clone(&shared);
        let handle = std::thread::spawn(move || run_loop(backend, &rx, &thread_shared));
        Self {
            tx,
            shared,
            handle: Some(handle),
        }
    }

    /// Play the given FLAC bytes, replacing any current track.
    pub fn play(&self, bytes: Vec<u8>) {
        let _ = self.tx.send(Command::Play(bytes));
    }

    /// Pause the current track.
    pub fn pause(&self) {
        let _ = self.tx.send(Command::Pause);
    }

    /// Resume the current track.
    pub fn resume(&self) {
        let _ = self.tx.send(Command::Resume);
    }

    /// Stop and unload the current track.
    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    /// Toggle between playing and paused, based on the current snapshot.
    pub fn toggle(&self) {
        match self.snapshot().status() {
            PlaybackStatus::Playing => self.pause(),
            PlaybackStatus::Paused => self.resume(),
            PlaybackStatus::Idle | PlaybackStatus::Ended => {}
        }
    }

    /// A snapshot of the current playback state for rendering.
    #[must_use]
    pub fn snapshot(&self) -> PlaybackState {
        self.shared
            .lock()
            .map_or_else(|_| PlaybackState::default(), |guard| guard.clone())
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Quit);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// The playback thread's main loop: apply commands as they arrive, and poll for
/// progress/end-of-track on the idle timeout.
fn run_loop<B: AudioBackend>(
    mut backend: B,
    rx: &Receiver<Command>,
    shared: &Mutex<PlaybackState>,
) {
    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(command) => {
                if !apply_command(&mut backend, shared, command) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => poll_progress(&backend, shared),
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Apply one command to the backend and shared state. Returns `false` when the
/// thread should exit.
fn apply_command<B: AudioBackend>(
    backend: &mut B,
    shared: &Mutex<PlaybackState>,
    command: Command,
) -> bool {
    match command {
        Command::Play(bytes) => match backend.play(bytes) {
            Ok(total) => with_state(shared, |state| state.start(total)),
            Err(error) => with_state(shared, |state| state.fail(error)),
        },
        Command::Pause => {
            backend.pause();
            with_state(shared, PlaybackState::pause);
        }
        Command::Resume => {
            backend.resume();
            with_state(shared, PlaybackState::resume);
        }
        Command::Stop => {
            backend.stop();
            with_state(shared, PlaybackState::stop);
        }
        Command::Quit => {
            backend.stop();
            return false;
        }
    }
    true
}

/// Reflect the backend's progress into the shared state: mark the track ended
/// once it drains, otherwise advance the position.
fn poll_progress<B: AudioBackend>(backend: &B, shared: &Mutex<PlaybackState>) {
    with_state(shared, |state| {
        if state.status() == PlaybackStatus::Playing {
            if backend.is_finished() {
                state.mark_ended();
            } else {
                state.set_position(backend.position());
            }
        }
    });
}

/// Run `f` against the shared state, ignoring a poisoned lock.
fn with_state(shared: &Mutex<PlaybackState>, f: impl FnOnce(&mut PlaybackState)) {
    if let Ok(mut guard) = shared.lock() {
        f(&mut guard);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    /// A backend that records calls and returns scripted decode results.
    #[derive(Default)]
    struct FakeBackend {
        fail: bool,
        finished: bool,
        position: Duration,
        calls: Vec<&'static str>,
    }

    impl AudioBackend for FakeBackend {
        fn play(&mut self, _bytes: Vec<u8>) -> Result<Option<Duration>, String> {
            self.calls.push("play");
            if self.fail {
                Err("decode failed".to_owned())
            } else {
                Ok(Some(Duration::from_secs(2)))
            }
        }

        fn pause(&mut self) {
            self.calls.push("pause");
        }

        fn resume(&mut self) {
            self.calls.push("resume");
        }

        fn stop(&mut self) {
            self.calls.push("stop");
        }

        fn is_finished(&self) -> bool {
            self.finished
        }

        fn position(&self) -> Duration {
            self.position
        }
    }

    fn state() -> Mutex<PlaybackState> {
        Mutex::new(PlaybackState::new())
    }

    #[test]
    fn play_command_starts_playback() {
        let mut backend = FakeBackend::default();
        let shared = state();

        assert!(apply_command(
            &mut backend,
            &shared,
            Command::Play(vec![1, 2, 3])
        ));

        let snapshot = shared.lock().unwrap().clone();
        assert_eq!(snapshot.status(), PlaybackStatus::Playing);
        assert_eq!(snapshot.total(), Some(Duration::from_secs(2)));
        assert_eq!(backend.calls, ["play"]);
    }

    #[test]
    fn play_failure_records_error() {
        let mut backend = FakeBackend {
            fail: true,
            ..FakeBackend::default()
        };
        let shared = state();

        apply_command(&mut backend, &shared, Command::Play(vec![]));

        let snapshot = shared.lock().unwrap().clone();
        assert_eq!(snapshot.status(), PlaybackStatus::Idle);
        assert_eq!(snapshot.error(), Some("decode failed"));
    }

    #[test]
    fn pause_resume_stop_drive_backend_and_state() {
        let mut backend = FakeBackend::default();
        let shared = state();
        apply_command(&mut backend, &shared, Command::Play(vec![]));

        apply_command(&mut backend, &shared, Command::Pause);
        assert_eq!(shared.lock().unwrap().status(), PlaybackStatus::Paused);

        apply_command(&mut backend, &shared, Command::Resume);
        assert_eq!(shared.lock().unwrap().status(), PlaybackStatus::Playing);

        apply_command(&mut backend, &shared, Command::Stop);
        assert_eq!(shared.lock().unwrap().status(), PlaybackStatus::Idle);

        assert_eq!(backend.calls, ["play", "pause", "resume", "stop"]);
    }

    #[test]
    fn quit_command_stops_and_exits_loop() {
        let mut backend = FakeBackend::default();
        let shared = state();
        assert!(!apply_command(&mut backend, &shared, Command::Quit));
        assert_eq!(backend.calls, ["stop"]);
    }

    #[test]
    fn poll_marks_ended_when_drained() {
        let mut backend = FakeBackend {
            finished: true,
            ..FakeBackend::default()
        };
        let shared = state();
        apply_command(&mut backend, &shared, Command::Play(vec![]));

        poll_progress(&backend, &shared);
        assert_eq!(shared.lock().unwrap().status(), PlaybackStatus::Ended);
    }

    #[test]
    fn poll_advances_position_while_playing() {
        let mut backend = FakeBackend {
            position: Duration::from_millis(750),
            ..FakeBackend::default()
        };
        let shared = state();
        apply_command(&mut backend, &shared, Command::Play(vec![]));

        poll_progress(&backend, &shared);
        let snapshot = shared.lock().unwrap().clone();
        assert_eq!(snapshot.status(), PlaybackStatus::Playing);
        assert_eq!(snapshot.position(), Duration::from_millis(750));
    }
}
