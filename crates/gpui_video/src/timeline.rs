use std::time::Duration;

/// Controls the tradeoff between seek precision and latency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeekMode {
    /// Decode forward from the preceding keyframe to reach the requested time.
    Accurate,
    /// Stop at a nearby keyframe for a faster seek.
    #[default]
    KeyFrame,
}

/// A snapshot of the media timeline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaybackTimeline {
    position: Duration,
    duration: Option<Duration>,
    seekable: bool,
}

impl PlaybackTimeline {
    pub(crate) fn new(position: Duration, duration: Option<Duration>, seekable: bool) -> Self {
        Self {
            position: duration.map_or(position, |duration| position.min(duration)),
            duration,
            seekable,
        }
    }

    pub fn position(&self) -> Duration {
        self.position
    }

    pub fn duration(&self) -> Option<Duration> {
        self.duration
    }

    pub fn is_seekable(&self) -> bool {
        self.seekable
    }

    /// Returns playback progress in the inclusive range `0.0..=1.0`.
    pub fn progress(&self) -> Option<f64> {
        let duration = self.duration?;
        if duration.is_zero() {
            return Some(0.0);
        }
        Some((self.position.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0))
    }

    pub(crate) fn target_after(&self, amount: Duration) -> Duration {
        let target = self.position.saturating_add(amount);
        self.duration
            .map_or(target, |duration| target.min(duration))
    }

    pub(crate) fn target_before(&self, amount: Duration) -> Duration {
        self.position.saturating_sub(amount)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::PlaybackTimeline;

    #[test]
    fn progress_is_clamped_to_duration() {
        let timeline =
            PlaybackTimeline::new(Duration::from_secs(15), Some(Duration::from_secs(10)), true);

        assert_eq!(timeline.position(), Duration::from_secs(10));
        assert_eq!(timeline.progress(), Some(1.0));
    }

    #[test]
    fn skip_targets_do_not_cross_timeline_bounds() {
        let timeline =
            PlaybackTimeline::new(Duration::from_secs(4), Some(Duration::from_secs(10)), true);

        assert_eq!(
            timeline.target_before(Duration::from_secs(8)),
            Duration::ZERO
        );
        assert_eq!(
            timeline.target_after(Duration::from_secs(8)),
            Duration::from_secs(10)
        );
    }
}
