use std::sync::Arc;

pub(super) use super::super::scene_snapshot::OutputValidity;
use super::super::scene_snapshot::{SceneSnapshot, frame_tiles, pass_tiles, same_passes};
use gpui::{AtlasTile, MeshTexture3d, Scene3dFrame, SubtreeEffectPass, SubtreeLayer};

use super::RenderRegion;

pub(super) struct OutputKey {
    frame: Arc<Scene3dFrame>,
    region: RenderRegion,
    source: Option<SceneSnapshot>,
    second: Option<SceneSnapshot>,
    passes: Option<Arc<[SubtreeEffectPass]>>,
    pass_quad: Option<Vec<u8>>,
    generations: Vec<u64>,
}

impl OutputKey {
    pub(super) fn new(
        layer: &SubtreeLayer,
        region: RenderRegion,
        mut generation: impl FnMut(AtlasTile) -> Option<u64>,
    ) -> Option<Self> {
        let frame = layer.scene3d.as_ref()?;
        let uses_ui = frame
            .objects
            .iter()
            .any(|object| matches!(object.texture, MeshTexture3d::Subtree));
        let mut tiles = Vec::new();
        frame_tiles(frame, &mut tiles);
        if uses_ui && !pass_tiles(&layer.intermediate_effects, &mut tiles) {
            return None;
        }
        let (source, second) = if uses_ui {
            (
                Some(SceneSnapshot::new(&layer.scene, &mut generation)?),
                match &layer.second_scene {
                    Some(scene) => Some(SceneSnapshot::new(scene, &mut generation)?),
                    None => None,
                },
            )
        } else {
            (None, None)
        };
        Some(Self {
            frame: frame.clone(),
            region,
            source,
            second,
            passes: (uses_ui && !layer.intermediate_effects.is_empty())
                .then(|| layer.intermediate_effects.clone()),
            pass_quad: (uses_ui && !layer.intermediate_effects.is_empty()).then(|| {
                bytemuck::bytes_of(&super::EffectInstance::from(&layer.composite)).to_vec()
            }),
            generations: tiles
                .into_iter()
                .map(&mut generation)
                .collect::<Option<_>>()?,
        })
    }

    pub(super) fn matches(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.frame, &other.frame)
            && self.region == other.region
            && match (&self.source, &other.source) {
                (Some(a), Some(b)) => a.matches(b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.second, &other.second) {
                (Some(a), Some(b)) => a.matches(b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.passes, &other.passes) {
                (Some(a), Some(b)) => same_passes(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.generations == other.generations
            && self.pass_quad == other.pass_quad
    }
}

pub(super) struct Output {
    pub key: OutputKey,
    pub region: RenderRegion,
    pub texture: Option<wgpu::Texture>,
    pub validity: OutputValidity,
}

impl Output {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        key: OutputKey,
        region: RenderRegion,
        allocate: bool,
    ) -> Self {
        Self {
            key,
            region,
            texture: allocate.then(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("scene3d_retained_output"),
                    size: wgpu::Extent3d {
                        width: region.output_size[0],
                        height: region.output_size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            }),
            validity: OutputValidity::default(),
        }
    }

    pub(super) fn copy(
        &self,
        destination: &wgpu::Texture,
        encoder: &mut wgpu::CommandEncoder,
        restore: bool,
    ) {
        let mut surface = destination.as_image_copy();
        surface.origin = wgpu::Origin3d {
            x: self.region.origin[0],
            y: self.region.origin[1],
            z: 0,
        };
        let texture = self.texture.as_ref().expect("admitted output texture");
        let local = texture.as_image_copy();
        let (source, destination) = if restore {
            (local, surface)
        } else {
            (surface, local)
        };
        encoder.copy_texture_to_texture(source, destination, texture.size());
    }
}
