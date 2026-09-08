use gpui::{EffectShader, EffectUniforms, IntoElement, Point, Rgba, point, rgb};

use crate::{Effect, EffectStage, MaskedEffect, effect, masked_effect};

/// Procedural surface normals and highlight spread.
#[derive(Clone, Copy, Debug)]
pub struct MaterialSurface {
    /// Normal tilt along the horizontal and vertical axes, clamped to -1 through 1.
    pub tilt: Point<f32>,
    /// Surface curvature, clamped to 0 through 2. Zero produces a flat surface.
    pub curvature: f32,
    /// Highlight spread, clamped to 0 through 1.
    pub roughness: f32,
    /// Fine directional surface texture, clamped to 0 through 1.
    pub texture: f32,
}

impl Default for MaterialSurface {
    fn default() -> Self {
        Self {
            tilt: point(0., 0.),
            curvature: 0.6,
            roughness: 0.3,
            texture: 0.35,
        }
    }
}

/// A directional light in surface coordinates: right, down, and toward the viewer.
#[derive(Clone, Copy, Debug)]
pub struct MaterialLight {
    /// Direction toward the light. Zero and non-finite vectors use the default direction.
    pub direction: [f32; 3],
    /// Light RGB; alpha is ignored.
    pub color: Rgba,
    /// Direct illumination strength, clamped to 0 through 4.
    pub intensity: f32,
    /// Ambient illumination, clamped to 0 through 1.
    pub ambient: f32,
}

impl Default for MaterialLight {
    fn default() -> Self {
        Self {
            direction: [-0.4, -0.5, 1.],
            color: rgb(0xf1f5ff),
            intensity: 1.2,
            ambient: 0.55,
        }
    }
}

/// Angle-dependent foil coloring with independently configurable surface and light.
#[derive(Clone, Copy, Debug)]
pub struct HolographicOptions {
    /// Surface normals and texture.
    pub surface: MaterialSurface,
    /// Illumination configuration.
    pub light: MaterialLight,
    /// Spectral coloring strength, clamped to 0 through 1.
    pub iridescence: f32,
    /// Spectral pattern frequency, clamped to 0.1 through 8.
    pub scale: f32,
    /// Surface grain and spectral axis angle in radians.
    pub angle: f32,
    /// Blend with the source color, clamped to 0 through 1. Zero preserves the source.
    pub strength: f32,
}

impl Default for HolographicOptions {
    fn default() -> Self {
        Self {
            surface: MaterialSurface::default(),
            light: MaterialLight::default(),
            iridescence: 0.85,
            scale: 1.2,
            angle: -0.55,
            strength: 1.,
        }
    }
}

fn bounded(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

impl HolographicOptions {
    /// Packs normalized parameters for the material shaders. `base` supplies the color
    /// for solid and masked surfaces; image stages use the sampled image color instead.
    pub fn uniforms(self, base: Rgba) -> EffectUniforms {
        let surface = self.surface;
        let light = self.light;
        let mut direction = light.direction;
        if !direction.iter().all(|v| v.is_finite()) || direction == [0.; 3] {
            direction = MaterialLight::default().direction;
        }
        let magnitude = direction.iter().fold(0_f32, |a, v| a.max(v.abs()));
        direction = direction.map(|v| v / magnitude);
        let angle = if self.angle.is_finite() {
            self.angle
        } else {
            -0.55
        };
        let mut uniforms = EffectUniforms::default();
        uniforms.set_slot(
            0,
            [
                bounded(surface.tilt.x, -1., 1., 0.),
                bounded(surface.tilt.y, -1., 1., 0.),
                bounded(surface.curvature, 0., 2., 0.6),
                bounded(surface.roughness, 0., 1., 0.3),
            ],
        );
        uniforms.set_slot(
            1,
            [
                direction[0],
                direction[1],
                direction[2],
                bounded(light.intensity, 0., 4., 1.2),
            ],
        );
        uniforms.set_slot(
            2,
            [
                bounded(light.color.r, 0., 1., 1.),
                bounded(light.color.g, 0., 1., 1.),
                bounded(light.color.b, 0., 1., 1.),
                bounded(light.ambient, 0., 1., 0.55),
            ],
        );
        uniforms.set_slot(
            3,
            [
                bounded(self.iridescence, 0., 1., 0.85),
                bounded(self.scale, 0.1, 8., 1.2),
                bounded(surface.texture, 0., 1., 0.35),
                bounded(self.strength, 0., 1., 1.),
            ],
        );
        uniforms.set_slot(
            4,
            [base.r, base.g, base.b, base.a].map(|v| bounded(v, 0., 1., 0.)),
        );
        uniforms.set_slot(5, [angle.cos(), angle.sin(), 0., 0.]);
        uniforms
    }
}

/// Creates a styled foil surface. No animation frames are requested by the material.
pub fn holographic(base: impl Into<Rgba>, options: HolographicOptions) -> Effect {
    effect(holographic_shader()).uniforms(options.uniforms(base.into()))
}

/// Shades monochrome text or SVG content through its existing alpha mask.
pub fn holographic_masked<E: IntoElement>(
    element: E,
    base: impl Into<Rgba>,
    options: HolographicOptions,
) -> MaskedEffect<E::Element> {
    masked_effect(element, holographic_mask_shader()).uniforms(options.uniforms(base.into()))
}

impl EffectStage {
    /// Shades source RGB without changing alpha, layout, or hit testing.
    pub fn holographic(options: HolographicOptions) -> Self {
        Self::new(holographic_image_shader())
            .uniforms(options.uniforms(rgb(0xffffff)))
            .enabled(bounded(options.strength, 0., 1., 1.) > 0.)
    }
}

const SURFACE: &str = concat!(
    include_str!("shaders/holographic.wgsl"),
    "\nfn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return foil_shade(input, params, params.slots[4]); }",
);

/// Portable solid-surface shader; configure with [`HolographicOptions::uniforms`].
pub fn holographic_shader() -> EffectShader {
    EffectShader::wgsl(SURFACE)
}

/// Portable glyph/SVG mask shader; configure with [`HolographicOptions::uniforms`].
pub fn holographic_mask_shader() -> EffectShader {
    EffectShader::wgsl_mask(SURFACE)
}

/// Portable image shader that retains source alpha.
pub fn holographic_image_shader() -> EffectShader {
    EffectShader::wgsl_image(concat!(
        include_str!("shaders/holographic.wgsl"),
        "\nfn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return foil_shade(input, params, sample_effect_image(input, input.uv)); }",
    ))
}
