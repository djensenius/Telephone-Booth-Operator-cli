//! The playback state machine, kept free of any audio I/O so it can be unit
//! tested deterministically.

use std::time::Duration;

/// Where playback currently is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    /// Nothing is loaded.
    Idle,
    /// A track is loaded and advancing.
    Playing,
    /// A track is loaded but paused.
    Paused,
    /// The loaded track played to completion.
    Ended,
}

/// A snapshot of playback for the UI: the status, elapsed position, the total
/// duration (when known), and the last error message (when the most recent
/// load failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackState {
    status: PlaybackStatus,
    position: Duration,
    total: Option<Duration>,
    error: Option<String>,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            status: PlaybackStatus::Idle,
            position: Duration::ZERO,
            total: None,
            error: None,
        }
    }
}

impl PlaybackState {
    /// Create an idle state.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The current status.
    #[must_use]
    pub const fn status(&self) -> PlaybackStatus {
        self.status
    }

    /// The elapsed playback position.
    #[must_use]
    pub const fn position(&self) -> Duration {
        self.position
    }

    /// The total track duration, when the decoder could report it.
    #[must_use]
    pub const fn total(&self) -> Option<Duration> {
        self.total
    }

    /// The most recent load error, if the last attempt to play failed.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether a track is loaded (playing or paused).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(
            self.status,
            PlaybackStatus::Playing | PlaybackStatus::Paused
        )
    }

    /// Begin a freshly loaded track, clearing any prior error and position.
    pub fn start(&mut self, total: Option<Duration>) {
        self.status = PlaybackStatus::Playing;
        self.position = Duration::ZERO;
        self.total = total;
        self.error = None;
    }

    /// Pause an actively playing track (no-op otherwise).
    pub fn pause(&mut self) {
        if self.status == PlaybackStatus::Playing {
            self.status = PlaybackStatus::Paused;
        }
    }

    /// Resume a paused track (no-op otherwise).
    pub fn resume(&mut self) {
        if self.status == PlaybackStatus::Paused {
            self.status = PlaybackStatus::Playing;
        }
    }

    /// Stop and unload the current track, returning to idle.
    pub fn stop(&mut self) {
        self.status = PlaybackStatus::Idle;
        self.position = Duration::ZERO;
        self.total = None;
    }

    /// Mark the loaded track as having finished playing.
    pub fn mark_ended(&mut self) {
        if self.is_active() {
            self.status = PlaybackStatus::Ended;
            if let Some(total) = self.total {
                self.position = total;
            }
        }
    }

    /// Record a load failure, returning to idle and storing the message.
    pub fn fail(&mut self, message: String) {
        self.status = PlaybackStatus::Idle;
        self.position = Duration::ZERO;
        self.total = None;
        self.error = Some(message);
    }

    /// Update the elapsed position while a track is active.
    pub fn set_position(&mut self, position: Duration) {
        if self.is_active() {
            self.position = position;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_idle() {
        let state = PlaybackState::new();
        assert_eq!(state.status(), PlaybackStatus::Idle);
        assert_eq!(state.position(), Duration::ZERO);
        assert!(state.total().is_none());
        assert!(!state.is_active());
    }

    #[test]
    fn start_sets_playing_and_clears_error() {
        let mut state = PlaybackState::new();
        state.fail("boom".to_owned());
        assert!(state.error().is_some());

        state.start(Some(Duration::from_secs(3)));
        assert_eq!(state.status(), PlaybackStatus::Playing);
        assert_eq!(state.total(), Some(Duration::from_secs(3)));
        assert!(state.error().is_none());
        assert!(state.is_active());
    }

    #[test]
    fn pause_and_resume_only_toggle_active_track() {
        let mut state = PlaybackState::new();
        // Pause is a no-op while idle.
        state.pause();
        assert_eq!(state.status(), PlaybackStatus::Idle);

        state.start(None);
        state.pause();
        assert_eq!(state.status(), PlaybackStatus::Paused);
        state.resume();
        assert_eq!(state.status(), PlaybackStatus::Playing);
    }

    #[test]
    fn stop_returns_to_idle() {
        let mut state = PlaybackState::new();
        state.start(Some(Duration::from_secs(2)));
        state.set_position(Duration::from_secs(1));
        state.stop();
        assert_eq!(state.status(), PlaybackStatus::Idle);
        assert_eq!(state.position(), Duration::ZERO);
        assert!(state.total().is_none());
    }

    #[test]
    fn mark_ended_snaps_position_to_total() {
        let mut state = PlaybackState::new();
        state.start(Some(Duration::from_secs(5)));
        state.mark_ended();
        assert_eq!(state.status(), PlaybackStatus::Ended);
        assert_eq!(state.position(), Duration::from_secs(5));

        // Ending an idle player does nothing.
        let mut idle = PlaybackState::new();
        idle.mark_ended();
        assert_eq!(idle.status(), PlaybackStatus::Idle);
    }

    #[test]
    fn set_position_ignored_unless_active() {
        let mut state = PlaybackState::new();
        state.set_position(Duration::from_secs(1));
        assert_eq!(state.position(), Duration::ZERO);

        state.start(None);
        state.set_position(Duration::from_secs(1));
        assert_eq!(state.position(), Duration::from_secs(1));
    }
}
