use super::*;
use crate::{Material, MaterialTexture, Mesh, Object, PbrMaterial};
use gpui::{AtlasKey, AtlasTile, DevicePixels, Size};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

#[derive(Default)]
struct CpuAtlas {
    tiles: RefCell<HashMap<AtlasKey, AtlasTile>>,
    uploads: Cell<u32>,
}

impl PlatformAtlas for CpuAtlas {
    fn get_or_insert_with<'a>(
        &self,
        key: &AtlasKey,
        build: &mut dyn FnMut() -> anyhow::Result<Option<(Size<DevicePixels>, Cow<'a, [u8]>)>>,
    ) -> anyhow::Result<Option<AtlasTile>> {
        if let Some(tile) = self.tiles.borrow().get(key) {
            return Ok(Some(*tile));
        }
        let Some((size, bytes)) = build()? else {
            return Ok(None);
        };
        assert_eq!(
            bytes.len(),
            size.width.0 as usize * size.height.0 as usize * 4
        );
        let tile = AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 0,
                kind: gpui::AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(self.uploads.get()),
            padding: 0,
            bounds: gpui::Bounds::new(gpui::point(DevicePixels(0), DevicePixels(0)), size),
        };
        self.uploads.set(self.uploads.get() + 1);
        self.tiles.borrow_mut().insert(key.clone(), tile);
        Ok(Some(tile))
    }

    fn remove(&self, key: &AtlasKey) {
        assert!(self.tiles.borrow_mut().remove(key).is_some());
    }
}

fn textured_scene() -> Scene {
    let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_pixel(2, 2, image::Rgba([128, 128, 255, 255])),
    )]));
    Scene::new().object(Object::new(
        Mesh::plane(),
        Material::image(image.clone())
            .pbr(PbrMaterial::default())
            .normal_texture(MaterialTexture::new(image)),
    ))
}

#[test]
fn image_admission_deduplicates_slots_and_checks_resident_inputs() {
    let atlas = CpuAtlas::default();
    let mut images = ImageCache::default();
    let mut preparation = PreparationCache::with_capacity(3);
    assert_eq!(images.byte_limit(), None);
    images.set_byte_limit(Some(16));
    images.set_limits(
        ImageCacheLimits {
            max_idle_images: 4,
            max_idle_bytes: 64,
        },
        |id| remove_image(&atlas, id),
    );
    let first_scene = textured_scene();
    let second_scene = textured_scene();
    for scene in [&first_scene, &second_scene] {
        images
            .prepare(&mut preparation, scene, 1., 1024, &atlas)
            .unwrap();
    }
    assert_eq!(atlas.uploads.get(), 2);
    assert_eq!(images.usage().active_bytes, 16);
    let previous_usage = images.usage();
    let request = textured_scene().object(textured_scene().objects[0].clone());
    images.set_byte_limit(Some(31));
    let failure = images.prepare(&mut preparation, &request, 1., 1024, &atlas);
    assert!(matches!(
        failure,
        Err(PrepareError::Resource {
            object_index: 1,
            ..
        })
    ));
    assert_eq!(atlas.uploads.get(), 3);
    assert_eq!(atlas.tiles.borrow().len(), 2);
    assert_eq!(images.usage(), previous_usage);

    images.set_byte_limit(Some(32));
    let retained = images
        .prepare(&mut preparation, &request, 1., 1024, &atlas)
        .unwrap();
    assert_eq!(atlas.uploads.get(), 5);
    assert_eq!(images.usage().active_images, 2);
    assert_eq!(images.usage().active_bytes, 32);
    assert_eq!(images.usage().idle_bytes, 32);
    let accepted_usage = images.usage();
    images.set_byte_limit(Some(31));
    assert_eq!(images.usage(), accepted_usage);
    assert!(
        images
            .prepare(&mut preparation, &request, 1., 1024, &atlas)
            .is_err()
    );
    assert_eq!(atlas.uploads.get(), 5);
    assert_eq!(atlas.tiles.borrow().len(), 4);
    assert_eq!(images.usage(), accepted_usage);
    assert_eq!(retained.frame().objects.len(), 2);
    images.set_byte_limit(None);
    images
        .prepare(&mut preparation, &request, 1., 1024, &atlas)
        .unwrap();
    assert_eq!(atlas.uploads.get(), 5);

    images.set_byte_limit(Some(32));
    atlas.tiles.borrow_mut().clear();
    images.clear();
    assert_eq!(images.byte_limit(), Some(32));
    assert_eq!(images.usage(), ImageCacheUsage::default());
}

#[test]
fn zero_image_budget_allows_solid_and_culled_inputs_but_rejects_visible_images() {
    let atlas = CpuAtlas::default();
    let mut images = ImageCache::default();
    let mut preparation = PreparationCache::new();
    images.set_byte_limit(Some(0));
    let scene = Scene::new()
        .object(Object::new(
            Mesh::plane(),
            Material::color(gpui::white())
                .unlit(true)
                .normal_texture(MaterialTexture::new("inactive.png")),
        ))
        .object(textured_scene().objects[0].clone().position([100., 0., 0.]));
    let solid = images
        .prepare(&mut preparation, &scene, 1., 1024, &atlas)
        .unwrap();
    assert_eq!(solid.frame().objects.len(), 1);
    assert!(
        images
            .prepare(&mut preparation, &textured_scene(), 1., 1024, &atlas)
            .is_err()
    );
    assert_eq!(atlas.uploads.get(), 0);
    assert!(atlas.tiles.borrow().is_empty());
    assert_eq!(images.usage(), ImageCacheUsage::default());
}

#[test]
fn resolver_reuses_resident_tiles_and_refreshes_evicted_preparations() {
    let atlas = CpuAtlas::default();
    let mut images = ImageCache::default();
    images.set_limits(
        ImageCacheLimits {
            max_idle_images: 1,
            max_idle_bytes: 16,
        },
        |id| remove_image(&atlas, id),
    );
    let mut preparation = PreparationCache::with_capacity(2);
    let first_scene = textured_scene();
    let second_scene = textured_scene();
    let first = images
        .prepare(&mut preparation, &first_scene, 1., 1024, &atlas)
        .unwrap();
    images
        .prepare(&mut preparation, &second_scene, 1., 1024, &atlas)
        .unwrap();
    let reused = images
        .prepare(&mut preparation, &first_scene, 1., 1024, &atlas)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &reused));
    assert_eq!(atlas.uploads.get(), 2);
    assert_eq!(atlas.tiles.borrow().len(), 2);
    assert_eq!(
        images.usage(),
        ImageCacheUsage {
            idle_images: 1,
            idle_bytes: 16,
            active_images: 1,
            active_bytes: 16,
        }
    );

    images.set_limits(ImageCacheLimits::default(), |id| remove_image(&atlas, id));
    images
        .prepare(&mut preparation, &second_scene, 1., 1024, &atlas)
        .unwrap();
    let rebuilt = images
        .prepare(&mut preparation, &first_scene, 1., 1024, &atlas)
        .unwrap();
    assert_eq!(atlas.uploads.get(), 4);
    assert_eq!(atlas.tiles.borrow().len(), 1);
    assert!(!Arc::ptr_eq(&first, &rebuilt));
    let MeshTexture3d::Image(old) = first.frame().objects[0].texture else {
        panic!("missing image")
    };
    let MeshTexture3d::Image(new) = rebuilt.frame().objects[0].texture else {
        panic!("missing image")
    };
    assert_ne!(old, new);
}

#[test]
fn resolver_failure_rolls_back_new_tiles_without_losing_existing_residency() {
    let atlas = CpuAtlas::default();
    let mut images = ImageCache::default();
    images.set_limits(
        ImageCacheLimits {
            max_idle_images: 1,
            max_idle_bytes: 16,
        },
        |id| remove_image(&atlas, id),
    );
    let mut preparation = PreparationCache::with_capacity(3);
    let first_scene = textured_scene();
    let second_scene = textured_scene();
    images
        .prepare(&mut preparation, &first_scene, 1., 1024, &atlas)
        .unwrap();
    images
        .prepare(&mut preparation, &second_scene, 1., 1024, &atlas)
        .unwrap();
    let usage = images.usage();
    let new_scene = textured_scene();
    let invalid = new_scene.clone().object(Object::new(
        Mesh::plane(),
        Material::image("unresolved.png"),
    ));
    let error = images.prepare(&mut preparation, &invalid, 1., 1024, &atlas);
    assert!(matches!(
        error,
        Err(PrepareError::Resource {
            object_index: 1,
            ..
        })
    ));
    assert_eq!(atlas.uploads.get(), 3);
    assert_eq!(atlas.tiles.borrow().len(), 2);
    assert_eq!(images.usage(), usage);
    images
        .prepare(&mut preparation, &first_scene, 1., 1024, &atlas)
        .unwrap();
    assert_eq!(atlas.uploads.get(), 3);
    assert!(
        images
            .prepare(&mut preparation, &new_scene, 1., 1, &atlas)
            .is_err()
    );
    assert_eq!(atlas.uploads.get(), 3);
    images
        .prepare(&mut preparation, &new_scene, 1., 1024, &atlas)
        .unwrap();
    assert_eq!(atlas.uploads.get(), 4);
    assert_eq!(atlas.tiles.borrow().len(), 2);
}

#[test]
fn alternating_views_reactivate_idle_images_without_eviction() {
    let mut cache = ImageCache::default();
    let limits = ImageCacheLimits {
        max_idle_images: 2,
        max_idle_bytes: 256,
    };
    let mut removed = Vec::new();
    cache.set_limits(limits, |id| removed.push(id));
    for id in [ImageId(0), ImageId(1), ImageId(0), ImageId(1)] {
        cache.finish(BTreeMap::from([(id, 64)]), |id| removed.push(id));
    }
    assert!(removed.is_empty());
    assert_eq!(
        cache.usage(),
        ImageCacheUsage {
            idle_images: 1,
            idle_bytes: 64,
            active_images: 1,
            active_bytes: 64,
        }
    );
    cache.finish(BTreeMap::new(), |id| removed.push(id));
    assert!(removed.is_empty());
    assert_eq!(
        cache.usage(),
        ImageCacheUsage {
            idle_images: 2,
            idle_bytes: 128,
            ..Default::default()
        }
    );
    cache.finish(BTreeMap::from([(ImageId(0), 64), (ImageId(1), 64)]), |id| {
        removed.push(id)
    });
    assert_eq!(
        cache.usage(),
        ImageCacheUsage {
            active_images: 2,
            active_bytes: 128,
            ..Default::default()
        }
    );
    assert!(removed.is_empty());
}

#[test]
fn entry_and_payload_limits_evict_idle_images_by_recency() {
    let mut cache = ImageCache::default();
    let mut removed = Vec::new();
    cache.set_limits(
        ImageCacheLimits {
            max_idle_images: 2,
            max_idle_bytes: 128,
        },
        |id| removed.push(id),
    );
    for id in [ImageId(0), ImageId(1), ImageId(2), ImageId(0), ImageId(3)] {
        cache.finish(BTreeMap::from([(id, 64)]), |id| removed.push(id));
    }
    assert_eq!(removed, [ImageId(1)]);
    assert_eq!(cache.usage().idle_bytes, 128);
    cache.set_limits(
        ImageCacheLimits {
            max_idle_images: 1,
            max_idle_bytes: 128,
        },
        |id| removed.push(id),
    );
    assert_eq!(removed, [ImageId(1), ImageId(2)]);
    cache.set_limits(
        ImageCacheLimits {
            max_idle_images: 1,
            max_idle_bytes: 63,
        },
        |id| removed.push(id),
    );
    assert_eq!(removed, [ImageId(1), ImageId(2), ImageId(0)]);
    assert_eq!(
        cache.usage(),
        ImageCacheUsage {
            active_images: 1,
            active_bytes: 64,
            ..Default::default()
        }
    );
    cache.finish(BTreeMap::new(), |id| removed.push(id));
    assert_eq!(removed, [ImageId(1), ImageId(2), ImageId(0), ImageId(3)]);
}

#[test]
fn failed_preparation_releases_only_new_allocations() {
    let mut cache = ImageCache::default();
    let mut removed = Vec::new();
    cache.set_limits(
        ImageCacheLimits {
            max_idle_images: 1,
            max_idle_bytes: 64,
        },
        |id| removed.push(id),
    );
    cache.finish(BTreeMap::from([(ImageId(0), 64)]), |id| removed.push(id));
    cache.finish(BTreeMap::from([(ImageId(1), 64)]), |id| removed.push(id));
    let before = cache.usage();
    cache.abort(
        BTreeMap::from([(ImageId(0), 64), (ImageId(1), 64), (ImageId(2), 64)]),
        |id| removed.push(id),
    );
    assert_eq!(removed, [ImageId(2)]);
    assert_eq!(cache.usage(), before);
    cache.finish(BTreeMap::from([(ImageId(0), 64)]), |id| removed.push(id));
    assert_eq!(removed, [ImageId(2)]);
    cache.set_limits(ImageCacheLimits::default(), |id| removed.push(id));
    assert_eq!(removed, [ImageId(2), ImageId(1)]);
}

#[test]
fn combined_payload_above_u64_does_not_wrap_below_budget() {
    let mut cache = ImageCache::default();
    let mut removed = Vec::new();
    cache.set_limits(
        ImageCacheLimits {
            max_idle_images: 3,
            max_idle_bytes: u64::MAX,
        },
        |id| removed.push(id),
    );
    cache.finish(BTreeMap::from([(ImageId(0), u64::MAX)]), |id| {
        removed.push(id)
    });
    cache.finish(BTreeMap::from([(ImageId(1), 4)]), |id| removed.push(id));
    assert_eq!(cache.usage().idle_bytes, u64::MAX);
    cache.finish(BTreeMap::new(), |id| removed.push(id));
    assert_eq!(removed, [ImageId(0)]);
    assert_eq!(
        cache.usage(),
        ImageCacheUsage {
            idle_images: 1,
            idle_bytes: 4,
            ..Default::default()
        }
    );
}

#[test]
fn default_release_and_atlas_clear_preserve_active_inputs_and_limits() {
    let mut cache = ImageCache::default();
    let mut removed = Vec::new();
    cache.finish(BTreeMap::from([(ImageId(0), 64)]), |id| removed.push(id));
    cache.finish(BTreeMap::from([(ImageId(1), 64)]), |id| removed.push(id));
    assert_eq!(removed, [ImageId(0)]);
    cache.set_limits(
        ImageCacheLimits {
            max_idle_images: 1,
            max_idle_bytes: 64,
        },
        |id| removed.push(id),
    );
    cache.finish(BTreeMap::from([(ImageId(2), 64)]), |id| removed.push(id));
    assert_eq!(removed, [ImageId(0)]);
    cache.clear();
    cache.finish(BTreeMap::from([(ImageId(3), 64)]), |id| removed.push(id));
    cache.finish(BTreeMap::new(), |id| removed.push(id));
    assert_eq!(removed, [ImageId(0)]);
    assert_eq!(
        cache.usage(),
        ImageCacheUsage {
            idle_images: 1,
            idle_bytes: 64,
            ..Default::default()
        }
    );
    cache.set_limits(ImageCacheLimits::default(), |id| removed.push(id));
    assert_eq!(removed, [ImageId(0), ImageId(3)]);
}
