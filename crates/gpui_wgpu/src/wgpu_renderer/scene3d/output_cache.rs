use std::{cell::Cell, rc::Rc, sync::Arc};

use gpui::{
    AtlasTile, EffectQuad, MeshTexture3d, Scene, Scene3dFrame, SubtreeEffectPass, SubtreeLayer,
};

use super::RenderRegion;

pub(super) struct OutputKey {
    frame: Arc<Scene3dFrame>,
    region: RenderRegion,
    source: Option<Rc<Scene>>,
    second: Option<Rc<Scene>>,
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
        if uses_ui {
            if !scene_tiles(&layer.scene, &mut tiles)
                || layer
                    .second_scene
                    .as_ref()
                    .is_some_and(|scene| !scene_tiles(scene, &mut tiles))
                || !pass_tiles(&layer.intermediate_effects, &mut tiles)
            {
                return None;
            }
        }
        Some(Self {
            frame: frame.clone(),
            region,
            source: uses_ui.then(|| layer.scene.clone()),
            second: uses_ui.then(|| layer.second_scene.clone()).flatten(),
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
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.second, &other.second) {
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && match (&self.passes, &other.passes) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
            && self.generations == other.generations
            && self.pass_quad == other.pass_quad
    }
}

fn frame_tiles(frame: &Scene3dFrame, tiles: &mut Vec<AtlasTile>) {
    for object in frame.objects.iter() {
        if let MeshTexture3d::Image(tile) = object.texture {
            tiles.push(tile);
        }
        tiles.extend(
            [
                object.metallic_roughness_texture,
                object.emissive_texture,
                object.normal_texture,
                object.occlusion_texture,
            ]
            .into_iter()
            .flatten()
            .map(|texture| texture.tile),
        );
    }
}

fn effect_tiles(effect: &EffectQuad, tiles: &mut Vec<AtlasTile>) {
    tiles.extend(
        [
            effect.image_tile,
            effect.second_image_tile,
            effect.third_image_tile,
            effect.fourth_image_tile,
        ]
        .into_iter()
        .flatten(),
    );
}

fn pass_tiles(passes: &[SubtreeEffectPass], tiles: &mut Vec<AtlasTile>) -> bool {
    for pass in passes {
        if pass.feedback.is_some() || pass.particles.is_some() || pass.particle_transition.is_some()
        {
            return false;
        }
        tiles.extend(pass.images.iter().copied());
    }
    true
}

fn scene_tiles(scene: &Scene, tiles: &mut Vec<AtlasTile>) -> bool {
    let mut reusable = true;
    scene.visit(&mut |scene| {
        reusable &=
            scene.particles.is_empty() && scene.fluids.is_empty() && scene.surfaces.is_empty();
        tiles.extend(scene.monochrome_sprites.iter().map(|sprite| sprite.tile));
        tiles.extend(scene.subpixel_sprites.iter().map(|sprite| sprite.tile));
        tiles.extend(scene.polychrome_sprites.iter().map(|sprite| sprite.tile));
        for effect in &scene.effects {
            effect_tiles(effect, tiles);
        }
        for layer in &scene.subtree_layers {
            effect_tiles(&layer.composite, tiles);
            reusable &= pass_tiles(&layer.intermediate_effects, tiles);
            if let Some(frame) = &layer.scene3d {
                frame_tiles(frame, tiles);
            }
        }
    });
    reusable
}

#[derive(Default)]
pub(super) struct OutputValidity {
    submitted: Cell<bool>,
    encoded: Cell<bool>,
}

impl OutputValidity {
    pub(super) fn reusable(&self) -> bool {
        self.submitted.get()
    }
    pub(super) fn encoded(&self) {
        self.encoded.set(true);
    }
    pub(super) fn commit(&self, submitted: bool) {
        if self.encoded.replace(false) {
            self.submitted.set(submitted);
        }
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
