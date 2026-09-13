mod geometry;

use std::sync::Arc;

use anyhow::{Result, ensure};
use gpui::{Rgba, rgb};
use gpui_3d::{
    Material, Mesh, Object, PickBehavior, Scene3dMaterialBindingLimits, Scene3dMaterialProgram,
    Scene3dMaterialSource, Scene3dMaterialValue, WgpuContext,
};

/// A constant-width luminous strand following an open, smooth 3D path.
/// Retain it between frames. Sampling updates uniforms and shares the mesh and shader.
/// Uses a native WGPU material; viewports containing it require GPU picking.
///
/// ```no_run
/// use gpui_3d::HeadlessRenderer;
/// use gpui_3d_effects::CurveLight;
///
/// let renderer = HeadlessRenderer::new()?;
/// let curve = CurveLight::new(renderer.context().clone(), [
///     [-1., -0.5, 0.], [0., 0.5, -0.5], [1., 0., 0.5],
/// ])?.width(0.02)?;
/// let moving_light = curve.object(0.4)?;
/// let partially_drawn = curve.reveal(0.6)?;
/// # Ok::<(), anyhow::Error>(())
/// ```
#[derive(Clone)]
pub struct CurveLight {
    source: Scene3dMaterialSource,
    path: Arc<[[f32; 3]]>,
    mesh: Mesh,
    color: Rgba,
    tail_length: f32,
}

impl CurveLight {
    /// Interpolates 2–4096 finite points with a centripetal Catmull–Rom spline.
    /// Consecutive duplicate points are removed. Coincident endpoints are rejected.
    /// The default diameter is 0.016 scene units, with eight sides per cross-section.
    pub fn new(context: WgpuContext, points: impl IntoIterator<Item = [f32; 3]>) -> Result<Self> {
        let path = geometry::smooth(points)?;
        let mesh = geometry::tube(&path, 0.016)?;
        Ok(Self {
            source: Scene3dMaterialSource::new(
                context,
                Scene3dMaterialProgram::compile(include_str!("shaders/curve_light.wgsl"))?,
            )?,
            path: path.into(),
            mesh,
            color: rgb(0x98e5f2),
            tail_length: 0.22,
        })
    }

    /// Sets the positive strand diameter in scene units, rebuilding its retained mesh.
    /// Object scaling also scales the strand's thickness.
    pub fn width(mut self, diameter: f32) -> Result<Self> {
        self.mesh = geometry::tube(&self.path, diameter)?;
        Ok(self)
    }

    /// Sets the sRGB strand color. Components must be finite and in [0, 1].
    pub fn color(mut self, color: impl Into<Rgba>) -> Self {
        self.color = color.into();
        self
    }

    /// Sets the fading tail's length as a fraction of the path, in (0, 1].
    pub fn tail_length(mut self, length: f32) -> Self {
        self.tail_length = length;
        self
    }

    /// Samples the bright head along the full path, from 0 to 1 by arc length.
    /// The dim strand remains visible. Progress is clamped, not automatically looped.
    pub fn object(&self, progress: f32) -> Result<Object> {
        self.sample(progress, 1.)
    }

    /// Draws the path up to progress with a bright leading edge and fading tail.
    /// Zero hides the entire strand; one reveals the full path. Decreasing progress
    /// retracts it without modifying geometry. The cut end has no additional cap.
    pub fn reveal(&self, progress: f32) -> Result<Object> {
        self.sample(progress, progress)
    }

    fn sample(&self, progress: f32, reveal: f32) -> Result<Object> {
        ensure!(progress.is_finite(), "curve light progress must be finite");
        ensure!(
            self.tail_length.is_finite() && self.tail_length > 0. && self.tail_length <= 1.,
            "curve light tail length must be in (0, 1]"
        );
        ensure!(
            [self.color.r, self.color.g, self.color.b, self.color.a]
                .into_iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(&v)),
            "curve light color must be in [0, 1]"
        );
        let data = [
            progress.clamp(0., 1.),
            reveal.clamp(0., 1.),
            self.tail_length,
            0.18,
        ];
        let bytes: Vec<u8> = data.into_iter().flat_map(f32::to_ne_bytes).collect();
        let snapshot = self.source.bind(
            [(0, Scene3dMaterialValue::Uniform(bytes.into()))],
            Scene3dMaterialBindingLimits::default(),
        )?;
        Ok(Object::new(
            self.mesh.clone(),
            Material::color(self.color)
                .unlit(true)
                .program(snapshot)
                .alpha_cutoff(0.01),
        )
        .pick_behavior(PickBehavior::Ignore)
        .cast_shadows(false)
        .receive_shadows(false))
    }
}
