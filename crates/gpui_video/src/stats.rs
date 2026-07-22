use std::sync::atomic::{AtomicU64, Ordering};

/// Cumulative frame-delivery statistics for one video player.
///
/// These counters cover the lifetime of the player and are intended for
/// diagnostics, adaptive quality decisions and performance overlays.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VideoPlaybackStats {
    decoded_frames: u64,
    delivered_frames: u64,
    dropped_frames: u64,
}

impl VideoPlaybackStats {
    /// Frames successfully converted from GStreamer samples.
    pub fn decoded_frames(self) -> u64 {
        self.decoded_frames
    }

    /// Frames delivered to the GPUI player entity.
    pub fn delivered_frames(self) -> u64 {
        self.delivered_frames
    }

    /// Stale frames discarded before delivery because a newer frame arrived.
    pub fn dropped_frames(self) -> u64 {
        self.dropped_frames
    }

    /// Fraction of decoded frames discarded by the latest-frame queue.
    pub fn drop_ratio(self) -> f64 {
        if self.decoded_frames == 0 {
            0.0
        } else {
            self.dropped_frames as f64 / self.decoded_frames as f64
        }
    }
}

#[derive(Default)]
pub(crate) struct PlaybackCounters {
    decoded_frames: AtomicU64,
    dropped_frames: AtomicU64,
}

impl PlaybackCounters {
    pub(crate) fn record_decoded_frame(&self) {
        self.decoded_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_dropped_frame(&self) {
        self.dropped_frames.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self, delivered_frames: u64) -> VideoPlaybackStats {
        VideoPlaybackStats {
            decoded_frames: self.decoded_frames.load(Ordering::Relaxed),
            delivered_frames,
            dropped_frames: self.dropped_frames.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::VideoPlaybackStats;

    #[test]
    fn drop_ratio_handles_empty_and_active_playback() {
        assert_eq!(VideoPlaybackStats::default().drop_ratio(), 0.0);

        let stats = VideoPlaybackStats {
            decoded_frames: 120,
            delivered_frames: 90,
            dropped_frames: 30,
        };
        assert_eq!(stats.drop_ratio(), 0.25);
    }
}
