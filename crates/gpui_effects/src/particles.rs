use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{Canvas, EffectHistoryId, ParticleFrame, canvas};
pub use gpui::{ParticlePhysics, ParticleSpawn};

/// Persistent clock and emission queue for a GPU particle surface.
/// Particle positions and velocities remain on the GPU.
pub struct Particles {
    id: EffectHistoryId,
    capacity: u32,
    generation: u64,
    frame: u64,
    time: Duration,
    expires: Duration,
    last_tick: Instant,
    paused: bool,
    pending: Vec<ParticleSpawn>,
    physics: ParticlePhysics,
}

impl Default for Particles {
    fn default() -> Self {
        Self::new(4096)
    }
}

impl Particles {
    /// Creates an empty system. Capacity is clamped to 1 through 65,536.
    pub fn new(capacity: u32) -> Self {
        Self {
            id: EffectHistoryId::new(),
            capacity: capacity.clamp(1, gpui::MAX_GPU_PARTICLES),
            generation: 0,
            frame: 0,
            time: Duration::ZERO,
            expires: Duration::ZERO,
            last_tick: Instant::now(),
            paused: false,
            pending: Vec::new(),
            physics: Default::default(),
        }
    }

    /// Queues an emission. Returns false while paused, for zero count, or if the
    /// per-frame command limit (32) has been reached. Notify the owning view after emission.
    pub fn emit(&mut self, mut spawn: ParticleSpawn) -> bool {
        if self.paused || spawn.count == 0 || self.pending.len() == gpui::MAX_PARTICLE_SPAWNS {
            return false;
        }
        spawn.count = spawn.count.min(self.capacity);
        spawn.lifetime.start = spawn.lifetime.start.max(Duration::from_millis(1));
        spawn.lifetime.end = spawn.lifetime.end.max(spawn.lifetime.start);
        self.pending.push(spawn);
        true
    }

    /// Current force parameters.
    pub fn physics(&self) -> ParticlePhysics {
        self.physics
    }

    /// Changes GPU force parameters without restarting particles.
    pub fn set_physics(&mut self, physics: ParticlePhysics) {
        self.physics = physics;
    }

    /// Freezes simulation and excludes paused time when resuming. Pending emissions are discarded.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.paused = paused;
            self.pending.clear();
            self.last_tick = Instant::now();
        }
    }

    /// Whether simulation is frozen.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Clears particles on the next paint, including while paused.
    pub fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.frame = self.frame.wrapping_add(1);
        self.expires = self.time;
        self.pending.clear();
    }

    /// Builds immutable paint input using wall-clock elapsed time.
    pub fn frame(&mut self) -> Arc<ParticleFrame> {
        self.advance(self.last_tick.elapsed())
    }

    /// Builds paint input using a supplied simulation delta instead of wall-clock time.
    pub fn advance(&mut self, elapsed: Duration) -> Arc<ParticleFrame> {
        self.last_tick = Instant::now();
        if !self.paused {
            self.time = self.time.saturating_add(elapsed);
            self.frame = self.frame.wrapping_add(1);
        }
        let pending = std::mem::take(&mut self.pending);
        for spawn in &pending {
            self.expires = self
                .expires
                .max(self.time.saturating_add(spawn.lifetime.end));
        }
        Arc::new(ParticleFrame {
            id: self.id,
            generation: self.generation,
            frame: self.frame,
            time: self.time,
            capacity: self.capacity,
            physics: self.physics,
            spawns: pending.into(),
            needs_animation: !self.paused && self.time < self.expires,
        })
    }
}

/// Builds a particle canvas. Set its size with `Styled` or place it inside an effect chain.
pub fn particles(state: &mut Particles) -> Canvas<()> {
    let frame = state.frame();
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| window.paint_particles(bounds, frame),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_clock_excludes_pauses_and_bounds_emissions() {
        let mut state = Particles::new(8);
        assert!(!state.advance(Duration::ZERO).needs_animation);
        let spawn = ParticleSpawn {
            count: 20,
            lifetime: Duration::from_secs(1)..Duration::from_secs(1),
            ..Default::default()
        };
        assert!(state.emit(spawn.clone()));
        let emitted = state.advance(Duration::ZERO);
        assert_eq!(emitted.spawns[0].count, 8);
        assert!(emitted.needs_animation);
        let running = state.advance(Duration::from_millis(200));
        assert!(running.spawns.is_empty());
        state.set_paused(true);
        assert!(!state.emit(spawn.clone()));
        let paused = state.advance(Duration::from_secs(20));
        assert_eq!(paused.time, running.time);
        assert_eq!(paused.frame, running.frame);
        assert!(!paused.needs_animation);
        state.clear();
        assert_ne!(state.advance(Duration::ZERO).generation, paused.generation);
        state.set_paused(false);
        for _ in 0..gpui::MAX_PARTICLE_SPAWNS {
            assert!(state.emit(spawn.clone()));
        }
        assert!(!state.emit(spawn));
        assert!(state.advance(Duration::ZERO).needs_animation);
        assert!(!state.advance(Duration::from_secs(1)).needs_animation);
    }
}
