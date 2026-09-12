//! Reusable GPUI audio and video playback components.
//!
//! A pluggable `MediaBackend` owns demuxing, video/audio decoding, audio
//! output and the shared playback clock. `VideoPlayer` renders only the
//! newest decoded video frame with GPUI's dynamic `surface` element. It
//! deliberately contains no controls, pointer behavior, status overlay or
//! fullscreen policy; host applications build those features from the
//! exported state and events.
//!
//! `AudioPlayer` is independent from the video backend boundary. It uses
//! Symphonia and CPAL consistently across platforms and exposes no built-in UI.

#[cfg(feature = "audio")]
mod audio;
#[cfg(feature = "video")]
mod video;

#[cfg(feature = "audio")]
pub use audio::{
    AudioInfo, AudioPlayer, AudioPlayerBuilder, AudioPlayerEvent, AudioPlayerOptions, AudioSource,
    AudioStreamHint, AudioStreamWriter,
};
pub use gpui_media_core::*;
#[cfg(feature = "video")]
pub use video::*;
