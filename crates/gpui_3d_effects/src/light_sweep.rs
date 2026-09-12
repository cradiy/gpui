use anyhow::{Result, ensure};
use gpui::{Rgba, rgb};
use gpui_3d::{
    MeshPass, MeshPassBlend, MeshPassDepth, MeshPassState, Scene3dMaterialBindingLimits,
    Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue, WgpuContext,
};

/// A soft, surface-tinted sheen moving across depth-writing surfaces in world space.
/// Retain one instance per GPU device; sampling shares its compiled shader.
///
/// ```no_run
/// use gpui_3d::{HeadlessRenderer, Material};
/// use gpui_3d_effects::LightSweep;
///
/// let renderer = HeadlessRenderer::new()?;
/// let sweep = LightSweep::new(renderer.context().clone())?
///     .color(gpui::rgb(0xc4f3ff))
///     .width(0.2)
///     .direction([1., 0.35, 0.])
///     .range([-1., 1.]);
/// let material = Material::color(gpui::rgb(0x527d94))
///     .mesh_passes([sweep.pass(0.5)?]);
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Clone)]
pub struct LightSweep {
    source: Scene3dMaterialSource,
    color: Rgba,
    intensity: f32,
    width: f32,
    direction: [f32; 3],
    range: [f32; 2],
}

impl LightSweep {
    /// Prepares the built-in shader on a window or headless renderer's context.
    pub fn new(context: WgpuContext) -> Result<Self> {
        Ok(Self {
            source: Scene3dMaterialSource::new(
                context,
                Scene3dMaterialProgram::compile(include_str!("shaders/light_sweep.wgsl"))?,
            )?,
            color: rgb(0xe5efff),
            intensity: 0.65,
            width: 0.24,
            direction: [1., 0., 0.],
            range: [-1., 1.],
        })
    }

    /// The owning device. Recreate the effect after device replacement or loss.
    pub fn context(&self) -> &WgpuContext {
        self.source.context()
    }

    /// Sets the sRGB light color. Alpha scales the added light.
    pub fn color(mut self, color: impl Into<Rgba>) -> Self {
        self.color = color.into();
        self
    }

    /// Sets nonnegative linear light strength; defaults to 0.65.
    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// Sets the positive band half-width in world units; defaults to 0.24.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets a nonzero world-space sweep direction, normalized during sampling.
    pub fn direction(mut self, direction: [f32; 3]) -> Self {
        self.direction = direction;
        self
    }

    /// Sets increasing bounds of surface positions projected onto the direction.
    /// The band starts and finishes outside these bounds. Defaults to [-1, 1].
    pub fn range(mut self, range: [f32; 2]) -> Self {
        self.range = range;
        self
    }

    /// Samples progress clamped to [0, 1], uploading one immutable parameter block.
    /// Attach the returned pass with `Material::mesh_passes`. Opaque and masked
    /// primary surfaces must write depth; blended surfaces are not supported.
    /// The pass adds color without changing geometry, shadows, or picking.
    /// Non-finite settings, zero directions, and invalid widths/ranges are errors.
    pub fn pass(&self, progress: f32) -> Result<MeshPass> {
        let values = [
            self.color.r,
            self.color.g,
            self.color.b,
            self.color.a,
            self.intensity,
            self.width,
            self.range[0],
            self.range[1],
            progress,
        ];
        ensure!(
            values.iter().chain(&self.direction).all(|v| v.is_finite()),
            "light sweep settings must be finite"
        );
        ensure!(
            [self.color.r, self.color.g, self.color.b, self.color.a]
                .into_iter()
                .all(|v| (0. ..=1.).contains(&v)),
            "light sweep color must be in [0, 1]"
        );
        ensure!(
            self.width > 0. && self.intensity >= 0. && self.range[0] < self.range[1],
            "light sweep requires positive width, nonnegative intensity, and increasing range"
        );
        let length = self
            .direction
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            .sqrt();
        ensure!(length > 0., "light sweep direction must be nonzero");
        let axis = self.direction.map(|v| (f64::from(v) / length) as f32);
        let start = f64::from(self.range[0]) - f64::from(self.width);
        let end = f64::from(self.range[1]) + f64::from(self.width);
        let center = (start + (end - start) * f64::from(progress.clamp(0., 1.))) as f32;
        ensure!(center.is_finite(), "light sweep center exceeds f32 range");
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let data = [
            axis[0],
            axis[1],
            axis[2],
            center,
            linear(self.color.r),
            linear(self.color.g),
            linear(self.color.b),
            self.intensity,
            self.width,
            self.color.a,
            0.,
            0.,
        ];
        let bytes: Vec<u8> = data.into_iter().flat_map(f32::to_ne_bytes).collect();
        let snapshot = self.source.bind(
            [(0, Scene3dMaterialValue::Uniform(bytes.into()))],
            Scene3dMaterialBindingLimits::default(),
        )?;
        Ok(MeshPass::new(snapshot).state(MeshPassState {
            depth_compare: MeshPassDepth::Equal,
            blend: MeshPassBlend::Additive,
            ..Default::default()
        }))
    }
}
