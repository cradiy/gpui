use std::time::Duration;

use anyhow::{Context, Result, ensure};

use crate::AnimationClip;

/// Caller-advanced clip time with independent pause, seek, speed and loop state.
/// This value retains only the clip range, not tracks, an instance or a clock.
#[derive(Clone, Debug)]
pub struct AnimationPlayback {
    start: Duration,
    duration: Duration,
    position: Duration,
    anchor: Duration,
    elapsed: Duration,
    rate: f64,
    looping: bool,
    playing: bool,
}

impl AnimationPlayback {
    /// Starts paused at the first authored key, at 1x speed without looping.
    pub fn new(clip: &AnimationClip) -> Self {
        Self {
            start: clip.start(),
            duration: clip.duration(),
            position: Duration::ZERO,
            anchor: Duration::ZERO,
            elapsed: Duration::ZERO,
            rate: 1.,
            looping: false,
            playing: false,
        }
    }

    /// Clip-relative position, including either endpoint.
    pub fn position(&self) -> Duration {
        self.position
    }
    /// Absolute authored time for transform and weight track sampling.
    pub fn time(&self) -> Duration {
        self.start + self.position
    }
    pub fn duration(&self) -> Duration {
        self.duration
    }
    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn is_looping(&self) -> bool {
        self.looping
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// Starts or resumes. A terminal endpoint restarts at the opposite endpoint
    /// for the current direction. A zero-duration clip remains paused.
    pub fn play(&mut self) {
        if self.playing || self.duration.is_zero() {
            return;
        }
        if self.rate > 0. && self.position == self.duration {
            self.position = Duration::ZERO;
        }
        if self.rate < 0. && self.position.is_zero() {
            self.position = self.duration;
        }
        self.rebase();
        self.playing = true;
    }

    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Pauses and seeks relative to the first key, clamping to the clip duration.
    pub fn seek(&mut self, position: Duration) {
        self.position = position.min(self.duration);
        self.playing = false;
        self.rebase();
    }

    /// Accepts finite nonzero rates, including negative rates for reverse playback.
    /// The current position and play/pause state are preserved on success or error.
    pub fn set_rate(&mut self, rate: f64) -> Result<()> {
        ensure!(
            rate.is_finite() && rate != 0.,
            "playback rate must be finite and nonzero"
        );
        if self.rate != rate {
            self.rate = rate;
            self.rebase();
        }
        Ok(())
    }

    pub fn set_looping(&mut self, looping: bool) {
        if self.looping != looping {
            self.looping = looping;
            self.rebase();
        }
    }

    /// Advances by caller-provided elapsed time. Returns whether position changed.
    /// Scaling uses total elapsed time since the latest control change, avoiding
    /// per-frame rounding accumulation. Overflow returns an error without mutation.
    pub fn advance(&mut self, elapsed: Duration) -> Result<bool> {
        if !self.playing || elapsed.is_zero() {
            return Ok(false);
        }
        let elapsed = self
            .elapsed
            .checked_add(elapsed)
            .context("playback elapsed time overflow")?;
        let scaled = if self.rate.abs() == 1. {
            elapsed
        } else {
            Duration::try_from_secs_f64(elapsed.as_secs_f64() * self.rate.abs())
                .context("scaled playback time overflow")?
        };
        if scaled.is_zero() {
            self.elapsed = elapsed;
            return Ok(false);
        }
        let length = self.duration.as_nanos();
        let anchor = self.anchor.as_nanos();
        let delta = scaled.as_nanos();
        let (position, finished) = if self.looping {
            let delta = delta % length;
            let position = if self.rate > 0. {
                (anchor + delta) % length
            } else {
                (anchor + length - delta) % length
            };
            (position, false)
        } else if self.rate > 0. {
            ((anchor + delta).min(length), delta >= length - anchor)
        } else {
            (anchor.saturating_sub(delta), delta >= anchor)
        };
        let position = Duration::new(
            (position / 1_000_000_000) as u64,
            (position % 1_000_000_000) as u32,
        );
        let changed = position != self.position;
        self.position = position;
        self.elapsed = elapsed;
        self.playing = !finished;
        Ok(changed)
    }

    fn rebase(&mut self) {
        self.anchor = self.position;
        self.elapsed = Duration::ZERO;
    }
}
