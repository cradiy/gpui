use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use gltf::json::{extensions::texture::TextureTransform, texture::Info};
use gpui::{RenderImage, Rgba};
use gpui_3d::{
    AlphaMode, Material, MaterialTexture, PbrMaterial, TextureAddressMode, TextureColorSpace,
    TextureFilter, TextureMipFilter, TextureSampling, TextureSlot, UvTransform,
};

use crate::{EncodedImage, PreparedDocument, PrimitiveGeometry};

/// One active material texture with its original glTF image and sampling semantics.
#[derive(Clone, Debug)]
pub struct TextureBinding {
    slot: TextureSlot,
    texture_index: usize,
    image_index: usize,
    image: EncodedImage,
    tex_coord_set: u32,
    sampling: TextureSampling,
}

impl TextureBinding {
    pub fn slot(&self) -> TextureSlot {
        self.slot
    }
    pub fn texture_index(&self) -> usize {
        self.texture_index
    }
    pub fn image_index(&self) -> usize {
        self.image_index
    }
    pub fn image(&self) -> &EncodedImage {
        &self.image
    }
    pub fn tex_coord_set(&self) -> u32 {
        self.tex_coord_set
    }
    pub fn sampling(&self) -> TextureSampling {
        self.sampling
    }
    /// RGB interpretation at sampling time; alpha remains linear.
    pub fn color_space(&self) -> TextureColorSpace {
        match self.slot {
            TextureSlot::BaseColor | TextureSlot::Emissive => TextureColorSpace::Srgb,
            _ => TextureColorSpace::Linear,
        }
    }
}

/// Validated material parameters and retained encoded-image dependencies.
#[derive(Clone)]
pub struct MaterialDefinition {
    index: Option<usize>,
    base: MaterialParameters,
    textures: Vec<TextureBinding>,
    tex_coord_set: Option<u32>,
}

#[derive(Clone, Copy)]
struct MaterialParameters {
    color: Rgba,
    pbr: PbrMaterial,
    cutoff: f32,
    alpha_mode: AlphaMode,
    double_sided: bool,
    unlit: bool,
    normal_scale: f32,
    occlusion_strength: f32,
}

impl MaterialParameters {
    fn resolve(self) -> Material {
        Material::color(self.color)
            .pbr(self.pbr)
            .alpha_cutoff(self.cutoff)
            .alpha_mode(self.alpha_mode)
            .double_sided(self.double_sided)
            .unlit(self.unlit)
            .normal_scale(self.normal_scale)
            .occlusion_strength(self.occlusion_strength)
    }
}

impl MaterialDefinition {
    /// None identifies the implicit glTF default material.
    pub fn index(&self) -> Option<usize> {
        self.index
    }
    pub fn textures(&self) -> &[TextureBinding] {
        &self.textures
    }
    /// The UV set required by every active map, or None for untextured materials.
    pub fn tex_coord_set(&self) -> Option<u32> {
        self.tex_coord_set
    }
    pub fn requires_tangents(&self) -> bool {
        self.textures
            .iter()
            .any(|texture| texture.slot == TextureSlot::Normal)
    }

    /// Checks active-map UV and tangent requirements without resolving images.
    /// Material overrides are allowed; source material indices need not match.
    pub fn validate_geometry(&self, geometry: &PrimitiveGeometry) -> Result<()> {
        ensure!(
            self.tex_coord_set.is_none() || self.tex_coord_set == geometry.tex_coord_set(),
            "material {:?}: mesh {} primitive {} has UV set {:?}, expected {:?}",
            self.index,
            geometry.mesh_index(),
            geometry.primitive_index(),
            geometry.tex_coord_set(),
            self.tex_coord_set
        );
        ensure!(
            !self.requires_tangents() || geometry.mesh().tangents().is_some(),
            "material {:?}: mesh {} primitive {} requires tangents",
            self.index,
            geometry.mesh_index(),
            geometry.primitive_index()
        );
        Ok(())
    }

    /// Resolves active images into a core material. The callback supplies straight-alpha
    /// BGRA pixels without applying RGB transfer conversions, UV flips, or premultiplication.
    /// Each image index is requested once per call; the caller owns decoding budgets/cache.
    pub fn resolve_images(
        &self,
        mut decode: impl FnMut(usize, &EncodedImage) -> Result<Arc<RenderImage>>,
    ) -> Result<Material> {
        let mut images = HashMap::new();
        let mut material = self.base.resolve();
        for binding in &self.textures {
            let result = (|| -> Result<()> {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    images.entry(binding.image_index)
                {
                    let image = decode(binding.image_index, &binding.image)?;
                    let size = image.size(0);
                    ensure!(
                        size.width.0 > 0 && size.height.0 > 0 && image.as_bytes(0).is_some(),
                        "decoded image has no nonempty first frame"
                    );
                    entry.insert(image);
                }
                Ok(())
            })();
            result.with_context(|| {
                format!(
                    "material {:?} {:?} texture {} image {}",
                    self.index, binding.slot, binding.texture_index, binding.image_index
                )
            })?;
            let image = images[&binding.image_index].clone();
            let texture = MaterialTexture::new(image).sampling(binding.sampling);
            material = match binding.slot {
                TextureSlot::BaseColor => material.base_color_texture(texture),
                TextureSlot::MetallicRoughness => material.metallic_roughness_texture(texture),
                TextureSlot::Emissive => material.emissive_texture(texture),
                TextureSlot::Normal => material.normal_texture(texture),
                TextureSlot::Occlusion => material.occlusion_texture(texture),
            };
        }
        Ok(material)
    }
}

impl PreparedDocument {
    /// Converts glTF factors and active map bindings without decoding images.
    /// None selects the default white, metallic, rough, opaque, single-sided material.
    pub fn material(&self, index: Option<usize>) -> Result<MaterialDefinition> {
        self.material_definition(index)
            .with_context(|| format!("material {index:?}"))
    }

    fn material_definition(&self, index: Option<usize>) -> Result<MaterialDefinition> {
        crate::validation::supported_extensions(self.gltf())?;
        let default = gltf::json::material::Material::default();
        let source = match index {
            Some(index) => self
                .gltf()
                .as_json()
                .materials
                .get(index)
                .context("material index out of range")?,
            None => &default,
        };
        let pbr = &source.pbr_metallic_roughness;
        let base_color = pbr.base_color_factor.0;
        for (name, values) in [
            ("baseColorFactor", base_color.as_slice()),
            ("emissiveFactor", source.emissive_factor.0.as_slice()),
        ] {
            ensure!(
                values
                    .iter()
                    .all(|v| v.is_finite() && (0. ..=1.).contains(v)),
                "invalid {name}"
            );
        }
        let parameters = PbrMaterial {
            metallic: pbr.metallic_factor.0,
            roughness: pbr.roughness_factor.0,
            emissive: source.emissive_factor.0,
        };
        ensure!(parameters.is_valid(), "invalid metallic/roughness factors");
        let cutoff = source.alpha_cutoff.map_or(0.5, |value| value.0);
        ensure!(cutoff.is_finite() && cutoff >= 0., "invalid alphaCutoff");
        let unlit = source
            .extensions
            .as_ref()
            .is_some_and(|extensions| extensions.unlit.is_some());
        let alpha_mode = match source.alpha_mode.unwrap() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        };
        let color = Rgba {
            r: srgb(base_color[0]),
            g: srgb(base_color[1]),
            b: srgb(base_color[2]),
            a: base_color[3],
        };
        let mut base = MaterialParameters {
            color,
            pbr: parameters,
            cutoff,
            alpha_mode,
            double_sided: source.double_sided,
            unlit,
            normal_scale: 1.,
            occlusion_strength: 1.,
        };
        let mut textures = Vec::new();
        let mut add = |slot, info: &Info| -> Result<()> {
            textures.push(
                self.texture_binding(
                    slot,
                    info.index.value(),
                    info.tex_coord,
                    info.extensions
                        .as_ref()
                        .and_then(|extension| extension.texture_transform.as_ref()),
                )
                .with_context(|| format!("{slot:?}"))?,
            );
            Ok(())
        };
        if let Some(info) = &pbr.base_color_texture {
            add(TextureSlot::BaseColor, info)?;
        }
        if !unlit {
            if let Some(info) = &pbr.metallic_roughness_texture {
                add(TextureSlot::MetallicRoughness, info)?;
            }
            if source.emissive_factor.0.iter().any(|&value| value != 0.)
                && let Some(info) = &source.emissive_texture
            {
                add(TextureSlot::Emissive, info)?;
            }
            if let Some(info) = &source.normal_texture {
                ensure!(
                    info.scale.is_finite() && info.scale >= 0.,
                    "unsupported normal scale"
                );
                base.normal_scale = info.scale;
                if info.scale != 0. {
                    let transform = transform_extension(
                        info.extensions
                            .as_ref()
                            .and_then(|extension| extension.others.get("KHR_texture_transform")),
                    )
                    .context("Normal KHR_texture_transform")?;
                    textures.push(
                        self.texture_binding(
                            TextureSlot::Normal,
                            info.index.value(),
                            info.tex_coord,
                            transform.as_ref(),
                        )
                        .context("Normal")?,
                    );
                }
            }
            if let Some(info) = &source.occlusion_texture {
                let strength = info.strength.0;
                ensure!(
                    strength.is_finite() && (0. ..=1.).contains(&strength),
                    "invalid occlusion strength"
                );
                base.occlusion_strength = strength;
                if strength != 0. {
                    let transform = transform_extension(
                        info.extensions
                            .as_ref()
                            .and_then(|extension| extension.others.get("KHR_texture_transform")),
                    )
                    .context("Occlusion KHR_texture_transform")?;
                    textures.push(
                        self.texture_binding(
                            TextureSlot::Occlusion,
                            info.index.value(),
                            info.tex_coord,
                            transform.as_ref(),
                        )
                        .context("Occlusion")?,
                    );
                }
            }
        }
        let tex_coord_set = textures.first().map(|binding| binding.tex_coord_set);
        for binding in &textures {
            ensure!(
                Some(binding.tex_coord_set) == tex_coord_set,
                "{:?} requires TEXCOORD_{} but other maps require TEXCOORD_{}; multiple active UV sets are unsupported",
                binding.slot,
                binding.tex_coord_set,
                tex_coord_set.unwrap()
            );
        }
        Ok(MaterialDefinition {
            index,
            base,
            textures,
            tex_coord_set,
        })
    }

    fn texture_binding(
        &self,
        slot: TextureSlot,
        texture_index: usize,
        tex_coord: u32,
        transform: Option<&TextureTransform>,
    ) -> Result<TextureBinding> {
        let texture = self
            .gltf()
            .textures()
            .nth(texture_index)
            .context("texture index out of range")?;
        let image_index = texture.source().index();
        let image = self
            .image(image_index)
            .context("image index out of range")?
            .clone();
        let sampling = sampling(texture.sampler(), transform)?;
        let tex_coord_set = transform
            .and_then(|transform| transform.tex_coord)
            .unwrap_or(tex_coord);
        Ok(TextureBinding {
            slot,
            texture_index,
            image_index,
            image,
            tex_coord_set,
            sampling,
        })
    }
}

fn transform_extension(value: Option<&serde_json::Value>) -> Result<Option<TextureTransform>> {
    value
        .map(|value| serde_json::from_value(value.clone()).context("invalid transform"))
        .transpose()
}

fn sampling(
    sampler: gltf::texture::Sampler<'_>,
    transform: Option<&TextureTransform>,
) -> Result<TextureSampling> {
    use gltf::texture::{MagFilter, MinFilter, WrappingMode};
    let mag = sampler.mag_filter().map(|filter| match filter {
        MagFilter::Nearest => TextureFilter::Nearest,
        MagFilter::Linear => TextureFilter::Linear,
    });
    let (filter, mip_filter) = match sampler.min_filter() {
        Some(MinFilter::Nearest) => (TextureFilter::Nearest, TextureMipFilter::None),
        Some(MinFilter::Linear) => (TextureFilter::Linear, TextureMipFilter::None),
        Some(MinFilter::NearestMipmapNearest) => {
            (TextureFilter::Nearest, TextureMipFilter::Nearest)
        }
        Some(MinFilter::LinearMipmapNearest) => (TextureFilter::Linear, TextureMipFilter::Nearest),
        Some(MinFilter::NearestMipmapLinear) => (TextureFilter::Nearest, TextureMipFilter::Linear),
        Some(MinFilter::LinearMipmapLinear) => (TextureFilter::Linear, TextureMipFilter::Linear),
        None => (
            mag.unwrap_or(TextureFilter::Linear),
            TextureMipFilter::Linear,
        ),
    };
    let address = |mode| match mode {
        WrappingMode::ClampToEdge => TextureAddressMode::Clamp,
        WrappingMode::Repeat => TextureAddressMode::Repeat,
        WrappingMode::MirroredRepeat => TextureAddressMode::Mirror,
    };
    Ok(TextureSampling {
        transform: transform
            .map(|transform| {
                UvTransform::from_scale_rotation_translation(
                    transform.scale.0,
                    transform.rotation.0,
                    transform.offset.0,
                )
            })
            .transpose()?
            .unwrap_or_default(),
        address_u: address(sampler.wrap_s()),
        address_v: address(sampler.wrap_t()),
        filter,
        mag_filter: mag,
        mip_filter,
        max_anisotropy: 1,
    })
}

fn srgb(linear: f32) -> f32 {
    if linear <= 0.0031308 {
        12.92 * linear
    } else {
        1.055 * linear.powf(1. / 2.4) - 0.055
    }
}
