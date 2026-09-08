use crate::{Pixels, Point, point, px};

/// Geometry and deterministic trajectories for a reversible particle transition.
#[derive(Clone, Copy, Debug)]
pub struct ParticleTransitionOptions {
    /// Source fragment spacing in logical pixels, clamped to 1 through 12.
    pub cell_size: Pixels,
    /// Main displacement toward the dispersed state, clamped to ±512 per axis.
    pub scatter: Point<Pixels>,
    /// Random displacement radius, clamped to 0 through 256 logical pixels.
    pub spread: Pixels,
    /// Luminous particle radius, clamped to 0.25 through 8 logical pixels.
    pub radius: Pixels,
    /// Maximum short trail length, clamped to 0 through 32 logical pixels.
    pub streak: Pixels,
    /// Stable seed for fragment departure times and trajectories.
    pub seed: u32,
}

impl Default for ParticleTransitionOptions {
    fn default() -> Self {
        Self {
            cell_size: px(3.),
            scatter: point(px(100.), px(-70.)),
            spread: px(60.),
            radius: px(0.85),
            streak: px(6.),
            seed: 7,
        }
    }
}

impl ParticleTransitionOptions {
    /// Clamps geometry to finite supported ranges.
    pub fn normalized(self) -> Self {
        fn value(v: Pixels, fallback: Pixels, low: f32, high: f32) -> Pixels {
            let v = f32::from(v);
            if v.is_finite() {
                px(v.clamp(low, high))
            } else {
                fallback
            }
        }
        let defaults = Self::default();
        Self {
            cell_size: value(self.cell_size, defaults.cell_size, 1., 12.),
            scatter: point(
                value(self.scatter.x, defaults.scatter.x, -512., 512.),
                value(self.scatter.y, defaults.scatter.y, -512., 512.),
            ),
            spread: value(self.spread, defaults.spread, 0., 256.),
            radius: value(self.radius, defaults.radius, 0.25, 8.),
            streak: value(self.streak, defaults.streak, 0., 32.),
            seed: self.seed,
        }
    }
}

/// Stateless fragmentation of a captured subtree into reversible particle paths.
#[derive(Clone, Copy, Debug)]
pub struct SubtreeParticleTransitionPass {
    /// Zero is the original source; one is fully dispersed and transparent.
    pub progress: f32,
    /// Fragment geometry and trajectories.
    pub options: ParticleTransitionOptions,
    /// Logical-to-device scale.
    pub scale_factor: f32,
}
