use std::{cell::Cell, rc::Rc, sync::Arc};

pub(super) use super::super::scene_snapshot::OutputValidity;
use super::super::scene_snapshot::{SceneSnapshot, frame_tiles, pass_tiles, same_passes};
use gpui::{AtlasTile, MeshTexture3d, Scene3dFrame, SubtreeEffectPass, SubtreeLayer};

use super::RenderRegion;

#[derive(Clone)]
pub(in crate::wgpu_renderer) struct OutputBudget(Rc<Cell<gpui::Scene3dOutputCacheStats>>);

impl Default for OutputBudget {
    fn default() -> Self {
        Self(Rc::new(Cell::new(gpui::Scene3dOutputCacheStats {
            budget_bytes: 64 * 1024 * 1024,
            ..Default::default()
        })))
    }
}

impl OutputBudget {
    pub(in crate::wgpu_renderer) fn stats(&self) -> gpui::Scene3dOutputCacheStats {
        self.0.get()
    }

    pub(in crate::wgpu_renderer) fn set_limit(&self, bytes: u64) -> bool {
        let mut stats = self.0.get();
        if stats.budget_bytes == bytes {
            return false;
        }
        stats.budget_bytes = bytes;
        self.0.set(stats);
        true
    }

    pub(super) fn reserve(&self, bytes: u64) -> Option<OutputReservation> {
        let mut stats = self.0.get();
        let retained = stats.retained_bytes.checked_add(bytes)?;
        if bytes == 0 || retained > stats.budget_bytes {
            return None;
        }
        stats.retained_textures = stats.retained_textures.checked_add(1)?;
        stats.retained_bytes = retained;
        self.0.set(stats);
        Some(OutputReservation {
            budget: self.clone(),
            bytes,
        })
    }
}

pub(super) struct OutputReservation {
    budget: OutputBudget,
    bytes: u64,
}

impl Drop for OutputReservation {
    fn drop(&mut self) {
        let mut stats = self.budget.0.get();
        stats.retained_bytes -= self.bytes;
        stats.retained_textures -= 1;
        self.budget.0.set(stats);
    }
}

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
    _reservation: Option<OutputReservation>,
}

impl Output {
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        key: OutputKey,
        region: RenderRegion,
        reservation: Option<OutputReservation>,
    ) -> Self {
        Self {
            key,
            region,
            texture: reservation.as_ref().map(|_| {
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
            _reservation: reservation,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_output_budget_is_shared_and_released_with_cache_ownership() {
        let root = OutputBudget::default();
        root.set_limit(1024);
        let capture = root.clone();
        let first = root.reserve(768).unwrap();
        assert!(capture.reserve(512).is_none());
        let second = capture.reserve(256).unwrap();
        assert_eq!(root.stats().retained_bytes, 1024);
        assert_eq!(root.stats().retained_textures, 2);
        assert_eq!(root.stats(), capture.stats());
        drop(first);
        assert_eq!(capture.stats().retained_bytes, 256);
        let replacement = capture.reserve(512).unwrap();
        drop(capture);
        assert_eq!(root.stats().retained_bytes, 768);
        drop(second);
        drop(replacement);
        assert_eq!(root.stats().retained_bytes, 0);
        assert_eq!(root.stats().retained_textures, 0);
    }

    #[test]
    fn scene3d_output_budget_rejects_disabled_overflowing_and_overcommitted_reservations() {
        let budget = OutputBudget::default();
        budget.set_limit(u64::MAX);
        let allocation = budget.reserve(u64::MAX).unwrap();
        assert!(budget.reserve(1).is_none());
        assert_eq!(budget.stats().retained_bytes, u64::MAX);
        assert_eq!(budget.stats().retained_textures, 1);
        budget.set_limit(1024);
        assert!(budget.reserve(1).is_none());
        drop(allocation);
        let allocation = budget.reserve(1024).unwrap();
        budget.set_limit(0);
        assert!(budget.reserve(1).is_none());
        drop(allocation);
        assert!(budget.reserve(0).is_none());
        assert!(budget.reserve(1).is_none());
        assert_eq!(budget.stats().retained_bytes, 0);
        budget.set_limit(2048);
        let allocation = budget.reserve(2048).unwrap();
        assert!(!budget.set_limit(2048));
        assert_eq!(budget.stats().retained_bytes, 2048);
        drop(allocation);
        assert_eq!(budget.stats().retained_textures, 0);
    }
}
