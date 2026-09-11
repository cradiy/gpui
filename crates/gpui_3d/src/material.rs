use crate::{AlphaMode, PbrMaterial, TextureColorSpace, TextureSampling};
use gpui::{ImageSource, Rgba};

#[derive(Clone)]
pub(crate) enum Texture {
    None,
    Image(ImageSource),
    Ui,
}

/// Semantic input slot used by a material's texture requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureSlot {
    /// Solid tint, image color, or captured UI input.
    BaseColor,
    /// Linear G roughness and B metallic multipliers.
    MetallicRoughness,
    /// sRGB emission multiplier.
    Emissive,
    /// Linear tangent-space normal input.
    Normal,
    /// Linear R indirect-light attenuation.
    Occlusion,
}

/// An image and its independent mesh-UV sampling configuration.
#[derive(Clone)]
pub struct MaterialTexture {
    pub(crate) image: ImageSource,
    pub(crate) sampling: TextureSampling,
    pub(crate) uv_set: u32,
}

impl MaterialTexture {
    /// Uses the first decoded frame with linear filtering and clamped coordinates.
    pub fn new(image: impl Into<ImageSource>) -> Self {
        Self {
            image: image.into(),
            sampling: TextureSampling::default(),
            uv_set: 0,
        }
    }

    /// Sets the UV transform, per-axis addressing, mip filtering and anisotropy.
    pub fn sampling(mut self, sampling: TextureSampling) -> Self {
        self.sampling = sampling;
        self
    }
    /// Selects a mesh coordinate set. Defaults to zero; active missing sets are errors.
    pub fn uv_set(mut self, set: u32) -> Self {
        self.uv_set = set;
        self
    }
}

mod mesh_pass;
pub use mesh_pass::*;

/// Solid or textured material with explicit alpha interpretation.
#[derive(Clone)]
pub struct Material {
    pub(crate) mesh_passes: std::sync::Arc<[gpui::MeshPass3d]>,
    pub(crate) custom_material: Option<gpui::MeshMaterial3d>,
    pub(crate) color: Rgba,
    pub(crate) texture: Texture,
    pub(crate) unlit: bool,
    pub(crate) alpha_cutoff: f32,
    pub(crate) alpha_mode: AlphaMode,
    pub(crate) double_sided: bool,
    pub(crate) sampling: TextureSampling,
    pub(crate) uv_set: u32,
    pub(crate) image_color_space: TextureColorSpace,
    pub(crate) pbr: Option<PbrMaterial>,
    pub(crate) metallic_roughness_texture: Option<MaterialTexture>,
    pub(crate) emissive_texture: Option<MaterialTexture>,
    pub(crate) normal_texture: Option<MaterialTexture>,
    pub(crate) normal_scale: f32,
    pub(crate) occlusion_texture: Option<MaterialTexture>,
    pub(crate) occlusion_strength: f32,
}
impl Material {
    /// Creates a lit solid material from an sRGB color.
    pub fn color(color: impl Into<Rgba>) -> Self {
        Self {
            mesh_passes: Default::default(),
            color: color.into(),
            custom_material: None,
            texture: Texture::None,
            unlit: false,
            alpha_cutoff: 0.5,
            alpha_mode: AlphaMode::Mask,
            double_sided: true,
            sampling: TextureSampling::default(),
            uv_set: 0,
            image_color_space: TextureColorSpace::default(),
            pbr: None,
            metallic_roughness_texture: None,
            emissive_texture: None,
            normal_texture: None,
            normal_scale: 1.,
            occlusion_texture: None,
            occlusion_strength: 1.,
        }
    }
    /// Replaces additional color draws. Rendering admits at most eight passes per mesh.
    /// Passes share geometry and standard inputs; primary shadow/data coverage is unchanged.
    pub fn mesh_passes(mut self, passes: impl IntoIterator<Item = MeshPass>) -> Self {
        self.mesh_passes = passes.into_iter().map(|pass| pass.0).collect();
        self
    }
    /// Uses a device-local material snapshot for color, shadow and data outputs.
    /// Custom coverage requires GPU picking; viewport CPU interaction is disabled.
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    pub fn program(mut self, snapshot: crate::Scene3dMaterialSnapshot) -> Self {
        self.custom_material = Some(gpui::MeshMaterial3d::new(std::sync::Arc::new(snapshot)));
        self
    }

    /// Restores built-in surface evaluation and lighting.
    pub fn builtin_program(mut self) -> Self {
        self.custom_material = None;
        self
    }

    /// Uses an image's first decoded frame, stretched over mesh UVs.
    pub fn image(image: impl Into<ImageSource>) -> Self {
        Self {
            texture: Texture::Image(image.into()),
            ..Self::color(gpui::white())
        }
    }
    /// Uses the viewport's decorative UI capture without lighting.
    pub fn ui() -> Self {
        Self {
            texture: Texture::Ui,
            unlit: true,
            ..Self::color(gpui::white())
        }
    }
    /// Sets an sRGB tint, decoded before multiplication; alpha follows the alpha mode.
    pub fn tint(mut self, color: impl Into<Rgba>) -> Self {
        self.color = color.into();
        self
    }
    /// Configures image textures. Captured UI textures retain their original
    /// UV mapping and linear edge-clamped sampling.
    pub fn image_sampling(mut self, sampling: TextureSampling) -> Self {
        self.sampling = sampling;
        self
    }
    /// Selects the base-color image's mesh coordinates. Captured UI uses set zero.
    pub fn image_uv_set(mut self, set: u32) -> Self {
        self.uv_set = set;
        self
    }
    /// Sets image RGB encoding. Alpha remains linear; solid tint and UI captures
    /// use sRGB regardless of this setting.
    pub fn image_color_space(mut self, color_space: TextureColorSpace) -> Self {
        self.image_color_space = color_space;
        self
    }
    /// Sets the base-color image and sampling while preserving tint, alpha,
    /// lighting, face visibility, other maps, and image RGB encoding.
    pub fn base_color_texture(mut self, texture: MaterialTexture) -> Self {
        self.texture = Texture::Image(texture.image);
        self.sampling = texture.sampling;
        self.uv_set = texture.uv_set;
        self
    }
    /// Enables metallic-roughness shading, preserving the base color, texture and cutoff.
    /// Rendering rejects invalid parameters. Unlit mode bypasses PBR, including emission.
    pub fn pbr(mut self, parameters: PbrMaterial) -> Self {
        self.pbr = Some(parameters);
        self
    }
    /// Multiplies PBR roughness by linear G and metallic by linear B; R and alpha
    /// are ignored. Only used when PBR is enabled and the material is lit.
    pub fn metallic_roughness_texture(mut self, texture: MaterialTexture) -> Self {
        self.metallic_roughness_texture = Some(texture);
        self
    }
    /// Multiplies PBR emission by sRGB RGB decoded before filtering. Alpha is
    /// ignored. Only used when PBR is enabled and the material is lit.
    pub fn emissive_texture(mut self, texture: MaterialTexture) -> Self {
        self.emissive_texture = Some(texture);
        self
    }
    /// Uses linear RGB tangent-space normals decoded from [0, 1] to [-1, 1].
    /// Alpha is ignored. Requires mesh tangents for this map's UV set when lit PBR
    /// and nonzero scale are enabled. Does not change geometry or picking normals.
    pub fn normal_texture(mut self, texture: MaterialTexture) -> Self {
        self.normal_texture = Some(texture);
        self
    }
    /// Scales normal-map XY before normalization. Defaults to 1; zero disables
    /// the map and its resource requests. Rendering rejects negative/non-finite values.
    pub fn normal_scale(mut self, scale: f32) -> Self {
        self.normal_scale = scale;
        self
    }

    /// Attenuates diffuse ambient and environment light using linear R.
    /// G, B and alpha are ignored. Works with basic and PBR lit materials.
    pub fn occlusion_texture(mut self, texture: MaterialTexture) -> Self {
        self.occlusion_texture = Some(texture);
        self
    }
    /// Blends from no occlusion at 0 to the full map at 1. Defaults to 1.
    /// Zero disables resource requests. Rendering rejects values outside [0, 1].
    pub fn occlusion_strength(mut self, strength: f32) -> Self {
        self.occlusion_strength = strength;
        self
    }

    pub(crate) fn lighting_textures(
        &self,
    ) -> impl Iterator<Item = (TextureSlot, &MaterialTexture)> {
        [
            (
                TextureSlot::MetallicRoughness,
                self.metallic_roughness_texture.as_ref(),
            ),
            (TextureSlot::Emissive, self.emissive_texture.as_ref()),
            (
                TextureSlot::Normal,
                self.normal_texture
                    .as_ref()
                    .filter(|_| self.normal_scale != 0.),
            ),
            (
                TextureSlot::Occlusion,
                self.occlusion_texture
                    .as_ref()
                    .filter(|_| self.occlusion_strength != 0.),
            ),
        ]
        .into_iter()
        .filter_map(|(slot, texture)| {
            texture
                .filter(|_| !self.unlit && (slot == TextureSlot::Occlusion || self.pbr.is_some()))
                .map(|texture| (slot, texture))
        })
    }
    /// Bypasses lighting, occlusion, and emission.
    pub fn unlit(mut self, unlit: bool) -> Self {
        self.unlit = unlit;
        self
    }
    /// Selects Opaque, Mask or Blend alpha interpretation. Defaults to Mask.
    pub fn alpha_mode(mut self, mode: AlphaMode) -> Self {
        self.alpha_mode = mode;
        self
    }
    /// Enables both faces for rendering and ray queries. Defaults to true.
    /// Single-sided materials keep the mesh's local counterclockwise front face,
    /// including under reflected node transforms.
    pub fn double_sided(mut self, double_sided: bool) -> Self {
        self.double_sided = double_sided;
        self
    }
    /// Sets a finite nonnegative Mask threshold and selects Mask mode.
    /// Zero accepts all alpha values; values above one reject the entire surface.
    pub fn alpha_cutoff(mut self, cutoff: f32) -> Self {
        assert!(cutoff.is_finite() && cutoff >= 0.);
        self.alpha_cutoff = cutoff;
        self.alpha_mode = AlphaMode::Mask;
        self
    }

    pub(crate) fn alpha_visible(&self, alpha: f32) -> bool {
        let alpha = alpha.clamp(0., 1.);
        match self.alpha_mode {
            AlphaMode::Opaque => true,
            AlphaMode::Mask => alpha >= self.alpha_cutoff,
            AlphaMode::Blend => alpha > 0.,
        }
    }
}
