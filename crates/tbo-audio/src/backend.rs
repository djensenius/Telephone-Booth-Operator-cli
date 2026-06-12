//! The audio output backend: a trait so the player's command handling can be
//! tested with a fake, plus the real [`RodioBackend`] driving the system's
//! default output device.

use std::io::Cursor;
use std::time::Duration;

use rodio::source::Source;
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};

/// A sink that plays decoded FLAC bytes on some output device.
///
/// Implementors own the decoding and playback; the player thread translates
/// commands into these calls and reflects the result in the shared state.
pub trait AudioBackend {
    /// Decode and begin playing `bytes`, replacing any current track. Returns
    /// the total duration when the decoder can report it.
    ///
    /// # Errors
    /// Returns a human-readable message when the bytes cannot be decoded.
    fn play(&mut self, bytes: Vec<u8>) -> Result<Option<Duration>, String>;

    /// Pause the current track.
    fn pause(&mut self);

    /// Resume the current track.
    fn resume(&mut self);

    /// Stop and unload the current track.
    fn stop(&mut self);

    /// Whether playback has drained (no track, or the track finished).
    fn is_finished(&self) -> bool;

    /// The elapsed position of the current track.
    fn position(&self) -> Duration;
}

/// A [`rodio`]-backed [`AudioBackend`] playing on the default output device.
///
/// Holds the output device open for the player's lifetime; dropping it stops
/// playback.
pub struct RodioBackend {
    device: MixerDeviceSink,
    player: Option<Player>,
}

impl RodioBackend {
    /// Open the system's default output device.
    ///
    /// # Errors
    /// Returns a message when no output device is available (e.g. a headless
    /// host) or the device cannot be opened.
    pub fn open() -> Result<Self, String> {
        let device = DeviceSinkBuilder::open_default_sink()
            .map_err(|err| format!("no audio output device: {err}"))?;
        Ok(Self {
            device,
            player: None,
        })
    }
}

impl AudioBackend for RodioBackend {
    fn play(&mut self, bytes: Vec<u8>) -> Result<Option<Duration>, String> {
        let decoder =
            Decoder::try_from(Cursor::new(bytes)).map_err(|err| format!("decode failed: {err}"))?;
        let total = decoder.total_duration();
        let player = Player::connect_new(self.device.mixer());
        player.append(decoder);
        self.player = Some(player);
        Ok(total)
    }

    fn pause(&mut self) {
        if let Some(player) = &self.player {
            player.pause();
        }
    }

    fn resume(&mut self) {
        if let Some(player) = &self.player {
            player.play();
        }
    }

    fn stop(&mut self) {
        if let Some(player) = self.player.take() {
            player.stop();
        }
    }

    fn is_finished(&self) -> bool {
        self.player.as_ref().is_none_or(rodio::Player::empty)
    }

    fn position(&self) -> Duration {
        self.player
            .as_ref()
            .map_or(Duration::ZERO, rodio::Player::get_pos)
    }
}
