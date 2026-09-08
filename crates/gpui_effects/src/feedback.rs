use std::time::{Duration, Instant};

use gpui::{EffectHistoryId, EffectShader, IntoElement, SubtreeFeedbackPass};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain, subtree_identity_shader};

/// Temporal decay and history resolution.
#[derive(Clone, Copy, Debug)]
pub struct FeedbackOptions {
    /// Time for an unwritten trail to disappear.
    pub fade_duration: Duration,
    /// History texture size divisor, clamped to 1 through 8.
    pub downsample: u32,
}

impl Default for FeedbackOptions {
    fn default() -> Self {
        Self {
            fade_duration: Duration::from_millis(1200),
            downsample: 2,
        }
    }
}

/// Persistent playback state for one feedback surface.
/// Keep this in the owning view and call `emit` when new input should be recorded.
pub struct Feedback {
    id: EffectHistoryId,
    options: FeedbackOptions,
    last_tick: Instant,
    time: Duration,
    last_emission: Option<Duration>,
    pending_capture: bool,
    paused: bool,
    generation: u64,
    frame: u64,
}

impl Default for Feedback {
    fn default() -> Self {
        Self::new(FeedbackOptions::default())
    }
}

impl Feedback {
    /// Creates an empty feedback surface.
    pub fn new(options: FeedbackOptions) -> Self {
        Self {
            id: EffectHistoryId::new(),
            options: FeedbackOptions {
                fade_duration: options.fade_duration.max(Duration::from_millis(1)),
                downsample: options.downsample.clamp(1, 8),
            },
            last_tick: Instant::now(),
            time: Duration::ZERO,
            last_emission: None,
            pending_capture: false,
            paused: false,
            generation: 0,
            frame: 0,
        }
    }

    /// Records the next stage input once. Calls made while paused are ignored.
    pub fn emit(&mut self) {
        if !self.paused {
            self.pending_capture = true;
        }
    }

    /// Clears retained pixels on the next paint, including while paused.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.frame = self.frame.wrapping_add(1);
        self.last_emission = None;
        self.pending_capture = false;
    }

    /// Freezes or resumes history. Paused time is excluded from decay.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.pending_capture = false;
            self.last_tick = Instant::now();
        }
    }

    /// Whether history is frozen.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Changes the fade duration. Values below one millisecond are clamped.
    pub fn set_fade_duration(&mut self, duration: Duration) {
        self.options.fade_duration = duration.max(Duration::from_millis(1));
    }

    /// Current fade duration.
    pub fn fade_duration(&self) -> Duration {
        self.options.fade_duration
    }

    /// Builds a stage using wall-clock elapsed time.
    /// Visible, unpaused trails request animation frames until they expire.
    pub fn stage(&mut self) -> EffectStage {
        self.advance(self.last_tick.elapsed())
    }

    /// Builds a stage using an application-supplied frame delta.
    /// Use this instead of `stage` when driving a simulation or playback clock.
    pub fn advance(&mut self, elapsed: Duration) -> EffectStage {
        self.last_tick = Instant::now();
        if !self.paused {
            self.time = self.time.saturating_add(elapsed);
            self.frame = self.frame.wrapping_add(1);
            if self
                .last_emission
                .is_some_and(|time| self.time.saturating_sub(time) >= self.options.fade_duration)
            {
                let pending = self.pending_capture;
                self.clear();
                self.pending_capture = pending;
            }
        }
        let capture = !self.paused && std::mem::take(&mut self.pending_capture);
        if capture {
            self.last_emission = Some(self.time);
        }
        let mut stage = EffectStage::new(subtree_identity_shader());
        stage.feedback = Some(SubtreeFeedbackPass {
            id: self.id,
            shader: feedback_shader(),
            generation: self.generation,
            frame: self.frame,
            time: self.time,
            fade_duration: self.options.fade_duration,
            capture,
            needs_animation: !self.paused && self.last_emission.is_some(),
            scale_factor: 1.,
            downsample: self.options.downsample,
        });
        stage
    }
}

/// Captures new input and composites it with this surface's decaying history.
pub fn subtree_feedback<E: IntoElement>(
    element: E,
    feedback: &mut Feedback,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [feedback.stage()])
}

/// Two-image feedback shader. Image one is new input; image two is retained history.
/// Slot 7: `[retention, capture_gain, alpha_cutoff, 0]`.
pub fn feedback_shader() -> EffectShader {
    EffectShader::wgsl_two_images(include_str!("shaders/feedback.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_clock_pauses_clears_and_expires() {
        let mut state = Feedback::new(FeedbackOptions {
            fade_duration: Duration::from_secs(1),
            ..Default::default()
        });
        let idle = state.advance(Duration::ZERO).feedback.unwrap();
        assert!(!idle.capture && !idle.needs_animation);
        state.emit();
        let emitted = state.advance(Duration::ZERO).feedback.unwrap();
        assert!(emitted.capture && emitted.needs_animation);
        let fading = state.advance(Duration::from_millis(200)).feedback.unwrap();
        assert!(!fading.capture && fading.needs_animation);
        state.set_paused(true);
        state.emit();
        let frozen = state.advance(Duration::from_secs(10)).feedback.unwrap();
        assert_eq!(frozen.time, fading.time);
        assert_eq!(frozen.frame, fading.frame);
        assert!(!frozen.capture && !frozen.needs_animation);
        state.clear();
        let cleared = state.advance(Duration::from_secs(10)).feedback.unwrap();
        assert_ne!(cleared.generation, frozen.generation);
        assert!(!cleared.capture && !cleared.needs_animation);
        state.set_paused(false);
        state.emit();
        let resumed = state.advance(Duration::ZERO).feedback.unwrap();
        assert_eq!(resumed.time, fading.time);
        assert!(resumed.capture && resumed.needs_animation);
        let expired = state.advance(Duration::from_secs(1)).feedback.unwrap();
        assert_ne!(expired.generation, resumed.generation);
        assert!(!expired.capture && !expired.needs_animation);
        state.emit();
        let next = state.advance(Duration::from_secs(3)).feedback.unwrap();
        assert!(next.capture && next.needs_animation);
    }
}
