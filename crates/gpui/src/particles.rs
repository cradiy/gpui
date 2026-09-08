use std::{ops::Range, sync::Arc, time::Duration};

use crate::{
    Bounds, ContentMask, EffectHistoryId, Pixels, Point, Rgba, ScaledPixels, point, px, rgb,
};

/// Maximum simultaneous particles in one GPU system.
pub const MAX_GPU_PARTICLES: u32 = 65_536;
/// Maximum emission commands in a frame.
pub const MAX_PARTICLE_SPAWNS: usize = 32;

/// Alpha-based emission from a captured element's painted pixels.
#[derive(Clone, Copy, Debug)]
pub struct ParticleMask {
    /// Minimum source alpha, clamped to 0.001 through 0.999.
    pub threshold: f32,
    /// Inward edge sampling band in logical pixels. Zero samples the full shape.
    pub edge_width: Pixels,
    /// Uses sampled RGB instead of the emission color; emission opacity still applies.
    pub inherit_color: bool,
}

impl Default for ParticleMask {
    fn default() -> Self {
        Self {
            threshold: 0.5,
            edge_width: px(2.),
            inherit_color: true,
        }
    }
}

/// A particle simulation emitted from the preceding subtree texture.
#[derive(Clone, Debug)]
pub struct SubtreeParticlePass {
    /// Persistent simulation input. Spawn line positions are replaced by mask samples.
    pub frame: Arc<ParticleFrame>,
    /// Selects source pixels and emission colors.
    pub mask: ParticleMask,
    /// Logical-to-device conversion for simulation and edge width.
    pub scale_factor: f32,
}

/// One emission distributed along a line in surface-local logical pixels.
#[derive(Clone, Debug)]
pub struct ParticleSpawn {
    /// First emission position.
    pub from: Point<Pixels>,
    /// Last emission position. Use the same position for a point burst.
    pub to: Point<Pixels>,
    /// Number of particles emitted.
    pub count: u32,
    /// Initial directional velocity in logical pixels per second.
    pub velocity: Point<Pixels>,
    /// Random radial speed in logical pixels per second.
    pub speed: Range<Pixels>,
    /// Random lifetime.
    pub lifetime: Range<Duration>,
    /// Random luminous core radius in logical pixels.
    pub radius: Range<Pixels>,
    /// Particle color and opacity.
    pub color: Rgba,
    /// Velocity-aligned tail duration in seconds. Zero produces round light points.
    pub stretch: f32,
}

impl Default for ParticleSpawn {
    fn default() -> Self {
        Self {
            from: point(px(0.), px(0.)),
            to: point(px(0.), px(0.)),
            count: 24,
            velocity: point(px(0.), px(0.)),
            speed: px(15.)..px(90.),
            lifetime: Duration::from_millis(700)..Duration::from_millis(1800),
            radius: px(0.8)..px(2.2),
            color: rgb(0xa0eaff),
            stretch: 0.025,
        }
    }
}

/// Forces evaluated on the GPU in surface-local logical coordinates.
#[derive(Clone, Copy, Debug)]
pub struct ParticlePhysics {
    /// Uniform acceleration in logical pixels per second squared.
    pub acceleration: Point<Pixels>,
    /// Exponential velocity damping per second.
    pub drag: f32,
    /// Center of the local force.
    pub attractor: Point<Pixels>,
    /// Positive values attract; negative values repel. Logical pixels per second squared.
    pub strength: Pixels,
    /// Local force support radius in logical pixels.
    pub radius: Pixels,
}

impl Default for ParticlePhysics {
    fn default() -> Self {
        Self {
            acceleration: point(px(0.), px(24.)),
            drag: 0.8,
            attractor: point(px(0.), px(0.)),
            strength: px(0.),
            radius: px(180.),
        }
    }
}

/// Immutable simulation input. Replaying the same frame does not advance particles twice.
#[derive(Clone, Debug)]
pub struct ParticleFrame {
    /// Persistent identity; one occurrence per rendered scene.
    pub id: EffectHistoryId,
    /// Changing this value clears the simulation.
    pub generation: u64,
    /// Monotonic update number.
    pub frame: u64,
    /// Simulation time, excluding pauses.
    pub time: Duration,
    /// Ring-buffer capacity, clamped to 1 through MAX_GPU_PARTICLES.
    pub capacity: u32,
    /// Simulation forces.
    pub physics: ParticlePhysics,
    /// New emissions. The first MAX_PARTICLE_SPAWNS commands are processed.
    pub spawns: Arc<[ParticleSpawn]>,
    /// Whether another animation frame is needed.
    pub needs_animation: bool,
}

/// A GPU particle system positioned within a scene.
#[derive(Clone, Debug)]
pub struct ParticleDraw {
    /// Scene draw order.
    pub order: crate::DrawOrder,
    /// Surface bounds in device pixels.
    pub bounds: Bounds<ScaledPixels>,
    /// Parent clip rectangle.
    pub content_mask: ContentMask<ScaledPixels>,
    /// Logical-to-device scale.
    pub scale_factor: f32,
    /// Final paint opacity; does not affect simulation state.
    pub opacity: f32,
    /// Simulation input.
    pub frame: Arc<ParticleFrame>,
}

impl From<ParticleDraw> for crate::Primitive {
    fn from(draw: ParticleDraw) -> Self {
        Self::Particles(draw)
    }
}
