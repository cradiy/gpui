use crate::{Bounds, Corners, EffectUniforms, Pixels, Point};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

const BACKDROP_SOURCE_MARKER: &str = "// __GPUI_BACKDROP_SOURCE__";
const BACKDROP_SAMPLING_MARKER: &str = "// __GPUI_BACKDROP_SAMPLING__";

/// Sampling implementation used by a renderer's backdrop shader.
#[doc(hidden)]
#[derive(Clone, Copy, Debug)]
pub enum BackdropSampling {
    /// Linear filtering through sampler binding 2.
    Hardware,
    /// Bilinear interpolation through texture loads, without a sampler binding.
    Manual,
}

/// Composes a complete portable WGSL module around a backdrop effect function.
#[doc(hidden)]
pub fn compose_backdrop_shader_wgsl(shader: &BackdropShader) -> String {
    compose_backdrop_shader_wgsl_with_sampling(shader, BackdropSampling::Hardware)
}

/// Composes a backdrop module with the renderer's sampling implementation.
#[doc(hidden)]
pub fn compose_backdrop_shader_wgsl_with_sampling(
    shader: &BackdropShader,
    sampling: BackdropSampling,
) -> String {
    let sampling = match sampling {
        BackdropSampling::Hardware => include_str!("backdrop_sampling.wgsl"),
        BackdropSampling::Manual => include_str!("backdrop_sampling_manual.wgsl"),
    };
    include_str!("backdrop.wgsl")
        .replace(BACKDROP_SAMPLING_MARKER, sampling)
        .replace(BACKDROP_SOURCE_MARKER, shader.wgsl_source())
}

/// Stable identifier derived from a backdrop shader's source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BackdropShaderId(u64);

impl BackdropShaderId {
    /// Returns the identifier as an integer suitable for renderer caches.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Portable fragment shader that samples scene content behind an element.
///
/// The WGSL source must define:
///
/// ```wgsl
/// fn backdrop_effect(input: BackdropInput, params: BackdropParams) -> vec4<f32>
/// ```
///
/// Implementations may call `sample_raw_backdrop` and
/// `sample_blurred_backdrop`, passing displacement in device pixels.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BackdropShader {
    id: BackdropShaderId,
    wgsl: Arc<str>,
}

impl BackdropShader {
    /// Creates a backdrop shader from its canonical WGSL function.
    pub fn wgsl(source: impl Into<Arc<str>>) -> Self {
        let wgsl = source.into();
        let mut hasher = DefaultHasher::new();
        wgsl.hash(&mut hasher);
        Self {
            id: BackdropShaderId(hasher.finish()),
            wgsl,
        }
    }

    /// Returns the stable identifier used by renderer pipeline caches.
    pub fn id(&self) -> BackdropShaderId {
        self.id
    }

    /// Returns the canonical WGSL backdrop function.
    pub fn wgsl_source(&self) -> &str {
        &self.wgsl
    }
}

/// A custom backdrop effect prepared for insertion into a window's paint scene.
#[derive(Clone, Debug)]
pub struct PaintBackdropEffect {
    /// Bounds of the affected region.
    pub bounds: Bounds<Pixels>,
    /// Radius used to prepare the blurred backdrop input.
    pub blur_radius: Pixels,
    /// Shader used to composite the raw and blurred backdrop inputs.
    pub shader: BackdropShader,
    /// User-defined shader parameters.
    pub uniforms: EffectUniforms,
    /// Animation time supplied to the shader.
    pub time: f32,
    /// Pointer position normalized relative to `bounds`.
    ///
    /// Values may be outside `0.0..=1.0` so shaders can react while the
    /// pointer approaches the effect.
    pub pointer: Point<f32>,
    /// Whether the pointer is currently inside the effect bounds.
    pub pointer_active: bool,
    /// Corner radii used to clip the result.
    pub corner_radii: Corners<Pixels>,
    /// Opacity applied after shader evaluation.
    pub opacity: f32,
}

impl PaintBackdropEffect {
    /// Creates a custom backdrop effect.
    pub fn new(
        bounds: impl Into<Bounds<Pixels>>,
        blur_radius: Pixels,
        shader: BackdropShader,
    ) -> Self {
        Self {
            bounds: bounds.into(),
            blur_radius,
            shader,
            uniforms: EffectUniforms::default(),
            time: 0.0,
            pointer: Point { x: 0.5, y: 0.5 },
            pointer_active: false,
            corner_radii: Corners::default(),
            opacity: 1.0,
        }
    }

    /// Replaces all user-defined uniform slots.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.uniforms = uniforms;
        self
    }

    /// Sets the shader animation time.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Sets the normalized pointer position and whether it is close enough to
    /// influence the material.
    pub fn pointer(mut self, pointer: Point<f32>, active: bool) -> Self {
        self.pointer = pointer;
        self.pointer_active = active;
        self
    }

    /// Sets the radii used to clip the effect.
    pub fn corner_radii(mut self, corner_radii: Corners<Pixels>) -> Self {
        self.corner_radii = corner_radii;
        self
    }

    /// Sets the compositing opacity.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backdrop_shader_identity_is_stable() {
        let source = "fn backdrop_effect() {}";
        assert_eq!(
            BackdropShader::wgsl(source).id(),
            BackdropShader::wgsl(source).id()
        );
        assert_ne!(
            BackdropShader::wgsl(source).id(),
            BackdropShader::wgsl("fn backdrop_effect_alt() {}").id()
        );
    }
}
