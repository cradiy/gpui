use anyhow::{Result, ensure};
use gpui::{Rgba, rgb};
use gpui_3d::{
    MeshPass, MeshPassBlend, MeshPassDepth, MeshPassState, Scene3dMaterialBindingLimits,
    Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue, WgpuContext,
};

/// Soft light at grazing view angles.
/// Smooth normals produce a rounded rim; hard normals retain visible face boundaries.
/// This adds light within the silhouette, not an extruded outline or cast light.
///
/// ```no_run
/// use gpui_3d::{HeadlessRenderer, Material};
/// use gpui_3d_effects::RimLight;
///
/// let renderer = HeadlessRenderer::new()?;
/// let rim = RimLight::new(renderer.context().clone())?
///     .color(gpui::rgb(0xc6e6ff))
///     .intensity(1.2)
///     .falloff(3.0);
/// let material = Material::color(gpui::rgb(0x527d94))
///     .mesh_passes([rim.pass()?]);
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Clone)]
pub struct RimLight {
    source: Scene3dMaterialSource,
    color: Rgba,
    intensity: f32,
    falloff: f32,
}

impl RimLight {
    /// Prepares the built-in shader on the supplied GPU context.
    pub fn new(context: WgpuContext) -> Result<Self> {
        Ok(Self {
            source: Scene3dMaterialSource::new(
                context,
                Scene3dMaterialProgram::compile(include_str!("shaders/rim_light.wgsl"))?,
            )?,
            color: rgb(0xc6e6ff),
            intensity: 1.2,
            falloff: 3.,
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

    /// Sets nonnegative linear light strength. Defaults to 1.2.
    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// Sets a positive angular falloff exponent. Larger values concentrate light
    /// closer to grazing angles; smaller values spread it across the surface.
    /// Defaults to 3. This is not a screen-space width.
    pub fn falloff(mut self, falloff: f32) -> Self {
        self.falloff = falloff;
        self
    }

    /// Creates an immutable additive pass. Retain it until settings change;
    /// camera and object movement use the renderer's current inputs automatically.
    /// Opaque and masked primary surfaces must write depth. Blended surfaces are
    /// not supported. Geometry, shadows, depth, normals, and picking are unchanged.
    /// Non-finite values, negative intensity, nonpositive falloff, or color
    /// components outside [0, 1] return an error.
    pub fn pass(&self) -> Result<MeshPass> {
        ensure!(
            self.intensity.is_finite()
                && self.intensity >= 0.
                && self.falloff.is_finite()
                && self.falloff > 0.,
            "rim light requires finite nonnegative intensity and positive falloff"
        );
        ensure!(
            [self.color.r, self.color.g, self.color.b, self.color.a]
                .into_iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(&v)),
            "rim light color must be finite and in [0, 1]"
        );
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let data = [
            linear(self.color.r),
            linear(self.color.g),
            linear(self.color.b),
            self.intensity,
            self.falloff,
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
