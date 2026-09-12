use std::sync::Arc;

use crate::MediaError;

/// Current high-level playback state shared by audio and video players.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Loading,
    Playing,
    Paused,
    Seeking,
    Ended,
    Error(Arc<MediaError>),
}
