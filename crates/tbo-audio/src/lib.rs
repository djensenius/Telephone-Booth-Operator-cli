//! In-terminal FLAC playback for `tb-operator`.
//!
//! Booth recordings and question prompts are FLAC blobs fetched from short-lived
//! SAS URLs. This crate plays those bytes on the operator's machine through a
//! small, UI-friendly handle:
//!
//! - [`AudioPlayer`] owns a background thread that holds the (thread-bound)
//!   [`rodio`] output device and plays one track at a time. The UI sends
//!   play/pause/resume/stop commands and reads a cloned [`PlaybackState`]
//!   snapshot each frame.
//! - [`PlaybackState`] / [`PlaybackStatus`] are the audio-free state machine,
//!   unit tested independently of any device.
//! - [`AudioBackend`] abstracts the output sink so playback logic can be tested
//!   with a fake; [`RodioBackend`] is the real implementation.
//!
//! Decoding is delegated to Symphonia via `rodio`'s `flac` feature; only FLAC is
//! enabled, since that is the only format the booth produces.

mod backend;
mod player;
mod state;

pub use backend::{AudioBackend, RodioBackend};
pub use player::AudioPlayer;
pub use state::{PlaybackState, PlaybackStatus};
