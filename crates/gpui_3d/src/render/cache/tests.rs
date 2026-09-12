use super::*;
use crate::{Material, Mesh, Object};

fn scene() -> Scene {
    Scene::new().object(Object::new(Mesh::cube(), Material::color(gpui::white())))
}

fn prepare(cache: &mut PreparationCache, scene: &Scene) -> Arc<PreparedScene> {
    cache
        .prepare(scene, 1., None, |_| {
            Ok(TextureState::Ready(MeshTexture3d::None))
        })
        .unwrap()
}

#[test]
fn alternating_cameras_reuse_entries_and_evict_the_least_recently_used() {
    let scene = scene();
    let side = scene.clone().camera(Camera::orbit(0.5, 0.2, 5.));
    let above = scene.clone().camera(Camera::orbit(0.5, 0.8, 5.));
    let mut cache = PreparationCache::with_capacity(2);
    let front = prepare(&mut cache, &scene);
    let first_side = prepare(&mut cache, &side);
    assert!(Arc::ptr_eq(&front, &prepare(&mut cache, &scene.clone())));
    let top = prepare(&mut cache, &above);
    assert!(Arc::ptr_eq(&front, &prepare(&mut cache, &scene)));
    let side_retained = prepare(&mut cache, &side);
    assert!(!Arc::ptr_eq(&first_side, &side_retained));
    assert_eq!(first_side.frame().objects.len(), 1);
    assert_eq!(top.frame().objects.len(), 1);

    cache.set_capacity(1);
    assert!(Arc::ptr_eq(&side_retained, &prepare(&mut cache, &side)));
    assert!(!Arc::ptr_eq(&front, &prepare(&mut cache, &scene)));
    cache.set_capacity(0);
    let bypass = prepare(&mut cache, &scene);
    assert!(!Arc::ptr_eq(&bypass, &prepare(&mut cache, &scene)));
    cache.set_capacity(2);
    let retained = prepare(&mut cache, &scene);
    cache.clear();
    assert!(!Arc::ptr_eq(&retained, &prepare(&mut cache, &scene)));
    assert_eq!(retained.frame().objects.len(), 1);
}

#[test]
fn dormant_entries_refresh_residency_before_reuse() {
    let scene = Scene::new().object(Object::new(Mesh::plane(), Material::image("surface.png")));
    let side = scene.clone().camera(Camera::orbit(0.5, 0.2, 5.));
    let mut cache = PreparationCache::with_capacity(2);
    let tile = gpui::AtlasTile {
        texture_id: gpui::AtlasTextureId {
            index: 0,
            kind: gpui::AtlasTextureKind::Polychrome,
        },
        tile_id: gpui::TileId(0),
        padding: 0,
        bounds: gpui::Bounds::new(
            gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
            gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(8)),
        ),
    };
    let mut calls = 0;
    let mut resolve = |_: TextureRequest<'_>| {
        calls += 1;
        Ok(TextureState::Ready(MeshTexture3d::Image(tile)))
    };
    let first = cache.prepare(&scene, 1., None, &mut resolve).unwrap();
    let other = cache.prepare(&side, 1., None, &mut resolve).unwrap();
    let hit = cache.prepare(&scene, 1., None, &mut resolve).unwrap();
    assert!(Arc::ptr_eq(&first, &hit));
    assert_eq!(calls, 3);
    let pending = cache
        .prepare(&scene, 1., None, |_| Ok(TextureState::Pending))
        .unwrap();
    assert!(!pending.is_ready());
    assert!(pending.frame().objects.is_empty());
    assert!(first.is_ready());
    assert_eq!(first.frame().objects.len(), 1);
    let moved_tile = gpui::AtlasTile {
        tile_id: gpui::TileId(1),
        ..tile
    };
    let refreshed = cache
        .prepare(&side, 1., None, |_| {
            Ok(TextureState::Ready(MeshTexture3d::Image(moved_tile)))
        })
        .unwrap();
    assert!(!Arc::ptr_eq(&other, &refreshed));
    assert!(
        matches!(refreshed.frame().objects[0].texture, MeshTexture3d::Image(value) if value == moved_tile)
    );
    assert!(
        matches!(other.frame().objects[0].texture, MeshTexture3d::Image(value) if value == tile)
    );
    let ready = cache
        .prepare(&scene, 1., None, |_| {
            Ok(TextureState::Ready(MeshTexture3d::Image(moved_tile)))
        })
        .unwrap();
    assert!(ready.is_ready());
    assert_eq!(ready.frame().objects.len(), 1);
}

#[test]
fn failed_preparations_preserve_unrelated_entries() {
    let scene = scene();
    let side = scene.clone().camera(Camera::orbit(0.5, 0.2, 5.));
    let mut cache = PreparationCache::with_capacity(2);
    let first = prepare(&mut cache, &scene);
    let other = prepare(&mut cache, &side);
    let error = cache.prepare(&scene, 1., None, |_| anyhow::bail!("resource unavailable"));
    assert!(matches!(
        error,
        Err(PrepareError::Resource {
            object_index: 0,
            ..
        })
    ));
    assert!(Arc::ptr_eq(&other, &prepare(&mut cache, &side)));
    let retry = prepare(&mut cache, &scene);
    assert!(!Arc::ptr_eq(&first, &retry));
    assert!(cache.prepare(&scene, 0., None, |_| unreachable!()).is_err());
    assert!(Arc::ptr_eq(&retry, &prepare(&mut cache, &scene)));
    assert!(Arc::ptr_eq(&other, &prepare(&mut cache, &side)));
}

#[test]
fn aspect_ui_configuration_and_content_select_independent_entries() {
    let scene = scene();
    let mut cache = PreparationCache::with_capacity(6);
    let configurations = [
        (1., None),
        (2., None),
        (
            1.,
            Some(UiTexture3d::new(
                gpui::size(gpui::px(80.), gpui::px(40.)),
                1.,
            )),
        ),
        (
            1.,
            Some(UiTexture3d::new(
                gpui::size(gpui::px(80.), gpui::px(40.)),
                2.,
            )),
        ),
        (
            1.,
            Some(UiTexture3d::new(
                gpui::size(gpui::px(40.), gpui::px(40.)),
                1.,
            )),
        ),
    ];
    let outputs: Vec<_> = configurations
        .iter()
        .map(|&(aspect, ui)| {
            cache
                .prepare(&scene, aspect, ui, |_| {
                    Ok(TextureState::Ready(MeshTexture3d::None))
                })
                .unwrap()
        })
        .collect();
    for (index, &(aspect, ui)) in configurations.iter().enumerate() {
        let hit = cache
            .prepare(&scene, aspect, ui, |_| {
                Ok(TextureState::Ready(MeshTexture3d::None))
            })
            .unwrap();
        for (other_index, output) in outputs.iter().enumerate() {
            assert_eq!(Arc::ptr_eq(&hit, output), index == other_index);
        }
    }
    let changed = scene
        .clone()
        .object(Object::new(Mesh::cube(), Material::color(gpui::white())));
    let updated = prepare(&mut cache, &changed);
    assert_eq!(updated.objects().len(), 2);
    assert_eq!(outputs[0].objects().len(), 1);
    assert!(Arc::ptr_eq(&outputs[0], &prepare(&mut cache, &scene)));
}
