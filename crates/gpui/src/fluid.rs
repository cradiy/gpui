use crate::{
    Bounds, ContentMask, EffectHistoryId, Pixels, Point, Rgba, ScaledPixels, point, px, rgb,
};
use std::{sync::Arc, time::Duration};

/// Maximum number of fluid injection commands in one update.
pub const MAX_FLUID_SPLATS: usize = 32;

/// A line-shaped injection in surface-local logical pixels.
#[derive(Clone, Copy, Debug)]
pub struct FluidSplat {
    /// Start of the injection segment.
    pub from: Point<Pixels>,
    /// End of the injection segment.
    pub to: Point<Pixels>,
    /// Added velocity in logical pixels per second.
    pub velocity: Point<Pixels>,
    /// Gaussian brush radius in logical pixels.
    pub radius: Pixels,
    /// Injected dye density, clamped to 0 through 4.
    pub amount: f32,
    /// Dye color. Alpha scales the injected density, not the velocity.
    pub color: Rgba,
}

impl Default for FluidSplat {
    fn default() -> Self {
        Self {
            from: point(px(0.), px(0.)),
            to: point(px(0.), px(0.)),
            velocity: point(px(0.), px(0.)),
            radius: px(24.),
            amount: 0.8,
            color: rgb(0x68dfff),
        }
    }
}

/// Grid resolution and solver parameters for a GPU fluid surface.
#[derive(Clone, Copy, Debug)]
pub struct FluidOptions {
    /// Longest grid dimension, clamped to 32 through 512. Aspect ratio follows the surface.
    pub resolution: u32,
    /// Simulation updates per second, clamped to 15 through 120.
    pub update_hz: u32,
    /// Jacobi pressure iterations per update, clamped to 8 through 60.
    pub pressure_iterations: u32,
    /// Exponential velocity damping per second.
    pub velocity_decay: f32,
    /// Exponential dye density decay per second. Zero retains dye indefinitely.
    pub dye_decay: f32,
    /// Vorticity confinement strength, clamped to 0 through 30.
    pub vorticity: f32,
}

impl Default for FluidOptions {
    fn default() -> Self {
        Self {
            resolution: 256,
            update_hz: 60,
            pressure_iterations: 24,
            velocity_decay: 0.35,
            dye_decay: 0.7,
            vorticity: 12.,
        }
    }
}

impl FluidOptions {
    /// Normalizes parameters to supported finite ranges.
    pub fn normalized(self) -> Self {
        Self {
            resolution: self.resolution.clamp(32, 512),
            update_hz: self.update_hz.clamp(15, 120),
            pressure_iterations: self.pressure_iterations.clamp(8, 60),
            velocity_decay: self.velocity_decay.max(0.).min(20.),
            dye_decay: self.dye_decay.max(0.).min(20.),
            vorticity: self.vorticity.max(0.).min(30.),
        }
    }
}

/// Immutable fluid update. Replaying a frame does not run the simulation twice.
#[derive(Clone, Debug)]
pub struct FluidFrame {
    /// Persistent identity; one occurrence per rendered scene.
    pub id: EffectHistoryId,
    /// Changing this value clears the fields.
    pub generation: u64,
    /// Monotonic simulation update number.
    pub frame: u64,
    /// Simulation time, excluding pauses.
    pub time: Duration,
    /// Solver configuration.
    pub options: FluidOptions,
    /// New injections. At most MAX_FLUID_SPLATS commands are processed.
    pub splats: Arc<[FluidSplat]>,
    /// Whether another animation frame is needed.
    pub needs_animation: bool,
}

/// A GPU fluid surface positioned within a scene.
#[derive(Clone, Debug)]
pub struct FluidDraw {
    /// Scene draw order.
    pub order: crate::DrawOrder,
    /// Surface bounds in device pixels.
    pub bounds: Bounds<ScaledPixels>,
    /// Parent clip rectangle.
    pub content_mask: ContentMask<ScaledPixels>,
    /// Logical-to-device scale.
    pub scale_factor: f32,
    /// Final paint opacity, independent of simulation state.
    pub opacity: f32,
    /// Simulation input.
    pub frame: Arc<FluidFrame>,
}

impl From<FluidDraw> for crate::Primitive {
    fn from(draw: FluidDraw) -> Self {
        Self::Fluid(draw)
    }
}
