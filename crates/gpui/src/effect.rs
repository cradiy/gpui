use crate::{Bounds, Corners, Pixels, RenderImage};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

/// Number of four-component floating-point uniform slots available to an effect.
pub const EFFECT_UNIFORM_SLOTS: usize = 8;

const EFFECT_SOURCE_MARKER: &str = "// __GPUI_EFFECT_SOURCE__";
const EFFECT_IMAGE_SOURCE_MARKER: &str = "// __GPUI_EFFECT_IMAGE_SOURCE__";

/// Composes a complete portable WGSL module around an effect function.
///
/// This is renderer-facing API. Applications normally provide only the
/// `effect` function through [`EffectShader::wgsl`].
#[doc(hidden)]
pub fn compose_effect_wgsl(effect_source: &str) -> String {
    compose_effect_wgsl_impl(effect_source, false)
}

/// Composes a complete portable WGSL module around an image effect function.
#[doc(hidden)]
pub fn compose_image_effect_wgsl(effect_source: &str) -> String {
    compose_effect_wgsl_impl(effect_source, true)
}

/// Composes the complete WGSL module represented by an effect shader.
#[doc(hidden)]
pub fn compose_effect_shader_wgsl(shader: &EffectShader) -> String {
    compose_effect_wgsl_impl(shader.wgsl_source(), shader.uses_image())
}

fn compose_effect_wgsl_impl(effect_source: &str, uses_image: bool) -> String {
    include_str!("effect.wgsl")
        .replace(
            EFFECT_IMAGE_SOURCE_MARKER,
            if uses_image {
                include_str!("effect_image.wgsl")
            } else {
                ""
            },
        )
        .replace(EFFECT_SOURCE_MARKER, effect_source)
}

/// Stable identifier derived from all source variants of an effect shader.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EffectShaderId(u64);

impl EffectShaderId {
    /// Returns the identifier as an integer suitable for renderer caches.
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// Shader model used when compiling a manually supplied HLSL implementation.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum HlslShaderModel {
    /// Shader Model 4.1, supported by Direct3D feature level 10.1.
    Sm4_1,
    /// Shader Model 5.0, supported by Direct3D feature level 11.0 and newer.
    #[default]
    Sm5_0,
}

/// Optional native HLSL implementation of a complete effect pipeline.
///
/// Native overrides must export `vs_effect` and `fs_effect` and follow the
/// renderer ABI used by [`compose_effect_wgsl`].
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct HlslEffectSource {
    source: Arc<str>,
    shader_model: HlslShaderModel,
}

impl HlslEffectSource {
    /// Creates a complete HLSL effect implementation targeting Shader Model 5.0.
    pub fn new(source: impl Into<Arc<str>>) -> Self {
        Self {
            source: source.into(),
            shader_model: HlslShaderModel::default(),
        }
    }

    /// Selects the shader model used to compile this implementation.
    pub fn shader_model(mut self, shader_model: HlslShaderModel) -> Self {
        self.shader_model = shader_model;
        self
    }

    /// Returns the complete HLSL source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns the requested HLSL shader model.
    pub fn model(&self) -> HlslShaderModel {
        self.shader_model
    }
}

#[derive(Debug, Eq, Hash, PartialEq)]
struct EffectShaderInner {
    id: EffectShaderId,
    wgsl: Arc<str>,
    msl: Option<Arc<str>>,
    hlsl: Option<HlslEffectSource>,
    uses_image: bool,
}

/// Portable fragment-effect source with optional native backend overrides.
///
/// WGSL is the required canonical implementation. MSL and HLSL sources, when
/// present, override generated native code on their respective renderers.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EffectShader(Arc<EffectShaderInner>);

impl EffectShader {
    /// Creates an effect shader from its canonical WGSL `effect` function.
    pub fn wgsl(source: impl Into<Arc<str>>) -> Self {
        Self::from_sources(source.into(), None, None, false)
    }

    /// Creates an image-sampling effect from its canonical WGSL `effect` function.
    ///
    /// Image effects may call `sample_effect_image` or
    /// `sample_effect_image_cover` from their WGSL implementation.
    pub fn wgsl_image(source: impl Into<Arc<str>>) -> Self {
        Self::from_sources(source.into(), None, None, true)
    }

    /// Adds a complete manually implemented Metal pipeline override.
    ///
    /// The source must export `vs_effect` and `fs_effect` and follow the
    /// renderer ABI used by [`compose_effect_wgsl`].
    pub fn with_msl(self, source: impl Into<Arc<str>>) -> Self {
        Self::from_sources(
            self.0.wgsl.clone(),
            Some(source.into()),
            self.0.hlsl.clone(),
            self.0.uses_image,
        )
    }

    /// Adds a complete manually implemented HLSL pipeline override.
    pub fn with_hlsl(self, source: HlslEffectSource) -> Self {
        Self::from_sources(
            self.0.wgsl.clone(),
            self.0.msl.clone(),
            Some(source),
            self.0.uses_image,
        )
    }

    /// Returns the stable shader identifier used by renderer pipeline caches.
    pub fn id(&self) -> EffectShaderId {
        self.0.id
    }

    /// Returns the canonical WGSL effect function.
    pub fn wgsl_source(&self) -> &str {
        &self.0.wgsl
    }

    /// Returns the optional manually implemented complete MSL effect pipeline.
    pub fn msl_source(&self) -> Option<&str> {
        self.0.msl.as_deref()
    }

    /// Returns the optional manually implemented complete HLSL effect pipeline.
    pub fn hlsl_source(&self) -> Option<&HlslEffectSource> {
        self.0.hlsl.as_ref()
    }

    /// Returns whether this shader samples an image texture.
    pub fn uses_image(&self) -> bool {
        self.0.uses_image
    }

    fn from_sources(
        wgsl: Arc<str>,
        msl: Option<Arc<str>>,
        hlsl: Option<HlslEffectSource>,
        uses_image: bool,
    ) -> Self {
        let mut hasher = DefaultHasher::new();
        wgsl.hash(&mut hasher);
        msl.hash(&mut hasher);
        hlsl.hash(&mut hasher);
        uses_image.hash(&mut hasher);
        let id = EffectShaderId(hasher.finish());
        Self(Arc::new(EffectShaderInner {
            id,
            wgsl,
            msl,
            hlsl,
            uses_image,
        }))
    }
}

/// Fixed-layout parameters passed to an effect shader.
///
/// Slots are represented as `vec4<f32>` on every backend to avoid differences
/// in native uniform-buffer layout rules.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct EffectUniforms {
    slots: [[f32; 4]; EFFECT_UNIFORM_SLOTS],
}

impl EffectUniforms {
    /// Creates zero-initialized effect parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets a four-component uniform slot.
    ///
    /// # Panics
    ///
    /// Panics when `index` is not smaller than [`EFFECT_UNIFORM_SLOTS`].
    pub fn with_slot(mut self, index: usize, value: [f32; 4]) -> Self {
        self.slots[index] = value;
        self
    }

    /// Updates a four-component uniform slot.
    ///
    /// # Panics
    ///
    /// Panics when `index` is not smaller than [`EFFECT_UNIFORM_SLOTS`].
    pub fn set_slot(&mut self, index: usize, value: [f32; 4]) {
        self.slots[index] = value;
    }

    /// Returns all uniform slots in their GPU layout.
    pub fn slots(&self) -> &[[f32; 4]; EFFECT_UNIFORM_SLOTS] {
        &self.slots
    }
}

/// A fragment effect prepared for insertion into a window's paint scene.
#[derive(Clone, Debug)]
pub struct PaintEffect {
    /// Bounds covered by the effect.
    pub bounds: Bounds<Pixels>,
    /// Shader used to calculate the color of each covered pixel.
    pub shader: EffectShader,
    /// User-defined effect parameters.
    pub uniforms: EffectUniforms,
    /// Animation time supplied to the effect function.
    pub time: f32,
    /// Corner radii used to clip the effect.
    pub corner_radii: Corners<Pixels>,
    /// Opacity applied after the effect function returns.
    pub opacity: f32,
    /// Optional image and frame sampled by an image effect.
    pub image: Option<(Arc<RenderImage>, usize)>,
}

impl PaintEffect {
    /// Creates an effect covering `bounds`.
    pub fn new(bounds: Bounds<Pixels>, shader: EffectShader) -> Self {
        Self {
            bounds,
            shader,
            uniforms: EffectUniforms::default(),
            time: 0.0,
            corner_radii: Corners::default(),
            opacity: 1.0,
            image: None,
        }
    }

    /// Sets the user-defined uniform slots.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.uniforms = uniforms;
        self
    }

    /// Sets the effect animation time.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Sets the radii used to clip the effect.
    pub fn corner_radii(mut self, corner_radii: Corners<Pixels>) -> Self {
        self.corner_radii = corner_radii;
        self
    }

    /// Sets the opacity applied after shader evaluation.
    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Sets the image frame sampled by this effect.
    pub fn image(mut self, image: Arc<RenderImage>, frame_index: usize) -> Self {
        self.image = Some((image, frame_index));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_variants_change_shader_identity() {
        let wgsl = EffectShader::wgsl("fn effect() {}");
        let msl = wgsl.clone().with_msl("float4 effect() {}");
        let hlsl = wgsl
            .clone()
            .with_hlsl(HlslEffectSource::new("float4 effect() {}"));
        let image = EffectShader::wgsl_image("fn effect() {}");

        assert_eq!(wgsl.id(), EffectShader::wgsl("fn effect() {}").id());
        assert_ne!(wgsl.id(), msl.id());
        assert_ne!(wgsl.id(), hlsl.id());
        assert_ne!(wgsl.id(), image.id());
        assert!(image.uses_image());
    }

    #[test]
    fn effect_uniform_slots_are_stable() {
        let uniforms = EffectUniforms::new().with_slot(3, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(uniforms.slots()[3], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(
            std::mem::size_of::<EffectUniforms>(),
            16 * EFFECT_UNIFORM_SLOTS
        );
    }
}
