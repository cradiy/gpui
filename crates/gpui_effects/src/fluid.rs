use gpui::{Canvas, EffectHistoryId, FluidFrame, canvas};
pub use gpui::{FluidOptions, FluidSplat};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// Persistent input queue and simulation clock for a GPU fluid surface.
pub struct Fluid {
    id: EffectHistoryId,
    options: FluidOptions,
    generation: u64,
    frame: u64,
    time: Duration,
    accumulated: Duration,
    expires: Duration,
    active: bool,
    paused: bool,
    last_tick: Instant,
    pending: Vec<FluidSplat>,
}
impl Default for Fluid {
    fn default() -> Self {
        Self::new(FluidOptions::default())
    }
}
impl Fluid {
    /// Creates an empty surface with a normalized solver configuration.
    pub fn new(options: FluidOptions) -> Self {
        Self {
            id: EffectHistoryId::new(),
            options: options.normalized(),
            generation: 0,
            frame: 0,
            time: Duration::ZERO,
            accumulated: Duration::ZERO,
            expires: Duration::ZERO,
            active: false,
            paused: false,
            last_tick: Instant::now(),
            pending: Vec::new(),
        }
    }
    /// Queues a line-shaped injection. Zero density applies force without adding color.
    /// Returns false while paused, for non-finite coordinates, or when 32 commands are queued.
    /// Notify the owning view after injecting.
    pub fn splat(&mut self, splat: FluidSplat) -> bool {
        if self.paused
            || self.pending.len() == gpui::MAX_FLUID_SPLATS
            || ![
                splat.from.x,
                splat.from.y,
                splat.to.x,
                splat.to.y,
                splat.velocity.x,
                splat.velocity.y,
                splat.radius,
            ]
            .iter()
            .all(|v| f32::from(*v).is_finite())
        {
            return false;
        }
        if !self.active && self.pending.is_empty() {
            self.last_tick = Instant::now();
            self.accumulated = Duration::ZERO;
        }
        self.pending.push(splat);
        true
    }
    /// Current normalized solver settings.
    pub fn options(&self) -> FluidOptions {
        self.options
    }
    /// Updates solver settings. Changing resolution clears the GPU fields.
    pub fn set_options(&mut self, options: FluidOptions) {
        let options = options.normalized();
        if options.resolution != self.options.resolution {
            self.clear();
        }
        self.options = options;
        if self.active {
            self.extend_lifetime();
        }
    }
    fn extend_lifetime(&mut self) {
        self.expires = if self.options.dye_decay == 0. {
            Duration::MAX
        } else {
            self.time.saturating_add(
                Duration::try_from_secs_f64(10. / f64::from(self.options.dye_decay))
                    .unwrap_or(Duration::MAX),
            )
        };
    }
    /// Freezes simulation, discards queued input, and excludes paused time on resume.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.pending.clear();
            self.accumulated = Duration::ZERO;
            self.last_tick = Instant::now();
        }
    }
    /// Whether simulation is frozen.
    pub fn is_paused(&self) -> bool {
        self.paused
    }
    /// Clears velocity and dye on the next paint, including while paused.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.frame = self.frame.wrapping_add(1);
        self.active = false;
        self.pending.clear();
        self.accumulated = Duration::ZERO;
    }
    /// Builds paint input from wall-clock time.
    pub fn frame(&mut self) -> Arc<FluidFrame> {
        self.advance(self.last_tick.elapsed())
    }
    /// Builds paint input from an explicit elapsed interval. Updates are capped at `update_hz`.
    /// After long gaps, transport advances at most 1/15 second; decay uses the full elapsed time.
    pub fn advance(&mut self, elapsed: Duration) -> Arc<FluidFrame> {
        self.last_tick = Instant::now();
        let mut splats = Vec::new();
        if !self.paused && (self.active || !self.pending.is_empty()) {
            self.accumulated = self.accumulated.saturating_add(elapsed);
            let interval = Duration::from_secs_f64(1. / f64::from(self.options.update_hz));
            if self.accumulated >= interval || !self.active {
                let remainder = if self.active {
                    Duration::from_nanos((self.accumulated.as_nanos() % interval.as_nanos()) as u64)
                } else {
                    Duration::ZERO
                };
                self.time = self.time.saturating_add(self.accumulated - remainder);
                self.accumulated = remainder;
                self.frame = self.frame.wrapping_add(1);
                splats = std::mem::take(&mut self.pending);
                if !splats.is_empty() {
                    self.active = true;
                    self.extend_lifetime();
                } else if self.time >= self.expires {
                    self.clear();
                }
            }
        }
        Arc::new(FluidFrame {
            id: self.id,
            generation: self.generation,
            frame: self.frame,
            time: self.time,
            options: self.options,
            splats: splats.into(),
            needs_animation: !self.paused && (self.active || !self.pending.is_empty()),
        })
    }
}

/// Creates a fluid canvas. Size it with `Styled` and compose it with subtree effects.
pub fn fluid(state: &mut Fluid) -> Canvas<()> {
    let frame = state.frame();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| window.paint_fluid(bounds, frame),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fluid_clock_throttles_input_and_excludes_paused_time() {
        let mut fluid = Fluid::new(FluidOptions {
            update_hz: 30,
            ..Default::default()
        });
        assert!(!fluid.advance(Duration::ZERO).needs_animation);
        assert!(fluid.splat(FluidSplat::default()));
        let initial = fluid.advance(Duration::ZERO);
        assert_eq!(initial.splats.len(), 1);
        for _ in 0..gpui::MAX_FLUID_SPLATS {
            assert!(fluid.splat(FluidSplat::default()));
        }
        assert!(!fluid.splat(FluidSplat::default()));
        let early = fluid.advance(Duration::from_millis(10));
        assert_eq!(early.frame, initial.frame);
        assert!(early.splats.is_empty());
        let next = fluid.advance(Duration::from_millis(24));
        assert_ne!(next.frame, initial.frame);
        assert_eq!(next.splats.len(), gpui::MAX_FLUID_SPLATS);
        fluid.set_paused(true);
        let paused = fluid.advance(Duration::from_secs(60));
        assert_eq!(paused.time, next.time);
        assert!(!paused.needs_animation);
        assert!(!fluid.splat(FluidSplat::default()));
        fluid.set_paused(false);
        let expired = fluid.advance(Duration::from_secs(20));
        assert!(!expired.needs_animation);
        assert_ne!(expired.generation, next.generation);
        fluid.set_options(FluidOptions {
            dye_decay: 0.,
            ..Default::default()
        });
        fluid.splat(FluidSplat::default());
        fluid.advance(Duration::ZERO);
        assert!(fluid.advance(Duration::from_secs(100)).needs_animation);
        fluid.set_paused(true);
        fluid.clear();
        assert!(!fluid.advance(Duration::ZERO).needs_animation);
    }
}
