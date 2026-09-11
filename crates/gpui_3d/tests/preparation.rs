use std::sync::Arc;

use gpui::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, TileId, UiTexture3d, point,
    px, rgb, size,
};
use gpui_3d::{
    Camera, Material, MaterialTexture, Mesh, Node, Object, PbrMaterial, PreparationCache,
    PrepareError, ResolvedTexture, Scene, SceneGraph, TextureSlot, TextureSource, TextureState,
};

fn tile() -> AtlasTile {
    AtlasTile {
        texture_id: AtlasTextureId {
            index: 2,
            kind: AtlasTextureKind::Polychrome,
        },
        tile_id: TileId(9),
        padding: 0,
        bounds: Bounds::new(
            point(DevicePixels(4), DevicePixels(8)),
            size(DevicePixels(16), DevicePixels(16)),
        ),
    }
}

#[test]
fn material_face_updates_preserve_retained_frames_and_mesh_sharing() {
    let mut graph = SceneGraph::new();
    let mesh = Mesh::plane();
    let material = Material::color(rgb(0xffffff));
    let node = graph
        .insert(None, Node::new().mesh(mesh, material.clone()))
        .unwrap();
    let old = graph.evaluate().unwrap().scene(Camera::default());
    let mut cache = PreparationCache::new();
    let prepared_old = cache
        .prepare(&old, 1., None, |_| {
            Ok(TextureState::Ready(ResolvedTexture::None))
        })
        .unwrap();
    graph
        .set_material(node, material.double_sided(false))
        .unwrap();
    let current = graph.evaluate().unwrap().scene(Camera::default());
    let prepared_new = cache
        .prepare(&current, 1., None, |_| {
            Ok(TextureState::Ready(ResolvedTexture::None))
        })
        .unwrap();
    assert!(prepared_old.frame().objects[0].double_sided);
    assert!(!prepared_new.frame().objects[0].double_sided);
    assert!(Arc::ptr_eq(
        &prepared_old.frame().objects[0].mesh,
        &prepared_new.frame().objects[0].mesh
    ));
}

fn layered_material() -> Material {
    Material::image("base.png")
        .pbr(PbrMaterial::default())
        .metallic_roughness_texture(MaterialTexture::new("surface.png"))
        .emissive_texture(MaterialTexture::new("emission.png"))
        .normal_texture(MaterialTexture::new("normal.png"))
        .occlusion_texture(MaterialTexture::new("occlusion.png"))
}

#[test]
fn sampling_validation_precedes_resource_resolution_and_preserves_active_configuration() {
    use gpui_3d::{TextureFilter, TextureMipFilter, TextureSampling};
    let scene = |material| Scene::new().object(Object::new(Mesh::plane(), material));
    for sampling in [
        TextureSampling {
            max_anisotropy: 0,
            ..Default::default()
        },
        TextureSampling {
            max_anisotropy: 17,
            mip_filter: TextureMipFilter::Linear,
            ..Default::default()
        },
        TextureSampling {
            max_anisotropy: 4,
            ..Default::default()
        },
        TextureSampling {
            max_anisotropy: 4,
            mip_filter: TextureMipFilter::Nearest,
            ..Default::default()
        },
        TextureSampling {
            max_anisotropy: 4,
            mip_filter: TextureMipFilter::Linear,
            filter: TextureFilter::Nearest,
            ..Default::default()
        },
        TextureSampling {
            max_anisotropy: 4,
            mip_filter: TextureMipFilter::Linear,
            mag_filter: Some(TextureFilter::Nearest),
            ..Default::default()
        },
    ] {
        for material in [
            Material::image("base.png").image_sampling(sampling),
            Material::color(rgb(0xffffff))
                .pbr(PbrMaterial::default())
                .normal_texture(MaterialTexture::new("normal.png").sampling(sampling)),
            Material::color(rgb(0xffffff))
                .occlusion_texture(MaterialTexture::new("ao.png").sampling(sampling)),
        ] {
            assert!(matches!(
                scene(material).prepare(1., None, |_| panic!(
                    "invalid sampling reached the resolver"
                )),
                Err(PrepareError::InvalidScene(_))
            ));
        }
    }
    let sampling = TextureSampling {
        mip_filter: TextureMipFilter::Linear,
        max_anisotropy: 16,
        ..Default::default()
    };
    let prepared = scene(Material::image("base.png").image_sampling(sampling))
        .prepare(1., None, |_| {
            Ok(TextureState::Ready(ResolvedTexture::Image(tile())))
        })
        .unwrap();
    assert_eq!(prepared.frame().objects[0].sampling, sampling);
    let invalid = TextureSampling {
        max_anisotropy: 0,
        ..Default::default()
    };
    let material = Material::color(rgb(0xffffff))
        .normal_texture(MaterialTexture::new("normal.png").sampling(invalid));
    let prepared = scene(material)
        .prepare(1., None, |request| {
            assert_eq!(request.slot, TextureSlot::BaseColor);
            Ok(TextureState::Ready(ResolvedTexture::None))
        })
        .unwrap();
    assert!(prepared.is_ready());
}

#[test]
fn pending_inputs_are_all_requested_and_do_not_shift_ready_object_ids() {
    let mut graph = SceneGraph::new();
    let node = graph
        .insert(
            None,
            Node::new()
                .id("textured")
                .mesh(Mesh::plane(), layered_material()),
        )
        .unwrap();
    graph
        .insert(
            None,
            Node::new()
                .id("solid")
                .mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default()).object(
        Object::new(Mesh::plane(), Material::image("distant.png"))
            .position([100., 0., 0.])
            .id("distant"),
    );
    let mut requests = Vec::new();
    let pending = scene
        .prepare(1., None, |request| {
            requests.push((request.object_index, request.slot));
            if request.object_index == 0 {
                assert_eq!(request.output_id, 1);
                assert_eq!(request.node, Some(node));
                assert_eq!(request.object_id, Some(&"textured".into()));
                assert!(matches!(request.source, TextureSource::Image(_)));
                Ok(TextureState::Pending)
            } else {
                assert!(matches!(request.source, TextureSource::Solid));
                Ok(TextureState::Ready(ResolvedTexture::None))
            }
        })
        .unwrap();
    assert_eq!(requests.len(), 6);
    for slot in [
        TextureSlot::BaseColor,
        TextureSlot::MetallicRoughness,
        TextureSlot::Emissive,
        TextureSlot::Normal,
        TextureSlot::Occlusion,
    ] {
        assert_eq!(
            requests
                .iter()
                .filter(|&&(index, s)| index == 0 && s == slot)
                .count(),
            1
        );
        assert!(
            pending
                .pending_textures()
                .iter()
                .any(|r| r.output_id == 1 && r.slot == slot)
        );
    }
    assert!(!pending.is_ready());
    assert_eq!(pending.pending_textures().len(), 5);
    assert_eq!(pending.frame().objects.len(), 1);
    assert_eq!(pending.frame().objects[0].output_id, 2);
    assert_eq!(pending.objects().len(), 3);
    assert_eq!(pending.object(3).unwrap().id, Some("distant".into()));
    assert!(pending.object(0).is_none() && pending.object(4).is_none());
    let ready = scene
        .prepare(1., None, |request| {
            Ok(TextureState::Ready(match request.source {
                TextureSource::Solid => ResolvedTexture::None,
                TextureSource::Image(_) => ResolvedTexture::Image(tile()),
                TextureSource::Ui => unreachable!(),
            }))
        })
        .unwrap();
    assert!(ready.is_ready());
    assert_eq!(
        ready
            .frame()
            .objects
            .iter()
            .map(|o| o.output_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(ready.object(1).unwrap().node, Some(node));
    assert_eq!(
        ready.frame().objects[0].normal_texture.unwrap().tile,
        tile()
    );
    graph.remove_subtree(node).unwrap();
    assert_eq!(ready.object(1).unwrap().node, Some(node));
    assert_eq!(pending.frame().objects.len(), 1);
    assert!(!pending.is_ready());
}

#[test]
fn resource_failures_keep_the_slot_and_underlying_error_separate_from_scene_validation() {
    let scene = Scene::new().object(Object::new(Mesh::plane(), layered_material()));
    let error = scene
        .prepare(1., None, |request| {
            if request.slot == TextureSlot::Normal {
                Err(
                    std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid image data")
                        .into(),
                )
            } else {
                Ok(TextureState::Ready(ResolvedTexture::Image(tile())))
            }
        })
        .unwrap_err();
    match error {
        PrepareError::Resource {
            object_index,
            slot,
            source,
        } => {
            assert_eq!(object_index, 0);
            assert_eq!(slot, TextureSlot::Normal);
            assert_eq!(
                source.downcast_ref::<std::io::Error>().unwrap().kind(),
                std::io::ErrorKind::InvalidData
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    assert!(matches!(
        scene.prepare(0., None, |_| unreachable!()),
        Err(PrepareError::InvalidScene(_))
    ));
    let invalid = Scene::new().object(
        Object::new(
            Mesh::plane(),
            Material::color(rgb(0xffffff)).normal_scale(f32::NAN),
        )
        .position([100., 0., 0.]),
    );
    assert!(matches!(
        invalid.prepare(1., None, |_| unreachable!()),
        Err(PrepareError::InvalidScene(_))
    ));
}

#[test]
fn resolved_kinds_match_sources_and_ui_dimensions_survive_preparation() {
    for (material, wrong) in [
        (
            Material::color(rgb(0xffffff)),
            ResolvedTexture::Image(tile()),
        ),
        (Material::image("base.png"), ResolvedTexture::Subtree),
        (Material::ui(), ResolvedTexture::None),
        (layered_material(), ResolvedTexture::None),
    ] {
        let error = Scene::new()
            .object(Object::new(Mesh::plane(), material))
            .prepare(1., None, |_| Ok(TextureState::Ready(wrong)))
            .unwrap_err();
        assert!(matches!(
            error,
            PrepareError::InvalidResolution {
                object_index: 0,
                ..
            }
        ));
    }
    let config = UiTexture3d::new(size(px(240.), px(120.)), 2.);
    let prepared = Scene::new()
        .object(Object::new(Mesh::plane(), Material::ui()))
        .prepare(2., Some(config), |request| {
            assert!(matches!(request.source, TextureSource::Ui));
            Ok(TextureState::Ready(ResolvedTexture::Subtree))
        })
        .unwrap();
    assert!(prepared.is_ready());
    assert_eq!(
        prepared.frame().ui_texture.unwrap().pixel_size(),
        size(DevicePixels(480), DevicePixels(240))
    );
    assert!(matches!(
        prepared.into_frame().objects[0].texture,
        ResolvedTexture::Subtree
    ));
}

#[test]
fn inactive_maps_and_culled_images_do_not_hold_readiness_open() {
    let scene = Scene::new()
        .object(Object::new(Mesh::plane(), layered_material().unlit(true)))
        .object(Object::new(Mesh::cube(), layered_material()).position([100., 0., 0.]));
    let mut count = 0;
    let prepared = scene
        .prepare(1., None, |request| {
            count += 1;
            assert_eq!(
                (request.object_index, request.slot),
                (0, TextureSlot::BaseColor)
            );
            Ok(TextureState::Ready(ResolvedTexture::Image(tile())))
        })
        .unwrap();
    assert_eq!(count, 1);
    assert!(prepared.is_ready());
    assert_eq!(prepared.objects().len(), 2);
    assert_eq!(prepared.frame().objects.len(), 1);
}

#[test]
fn retained_preparation_tracks_camera_aspect_ui_configuration_and_explicit_release() {
    let scene = Scene::new().object(Object::new(Mesh::plane(), Material::ui()).id("panel"));
    let mut cache = PreparationCache::new();
    let ui = UiTexture3d::new(size(px(200.), px(100.)), 1.);
    let resolve =
        |_: gpui_3d::TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::Subtree));
    let first = cache.prepare(&scene, 1., Some(ui), resolve).unwrap();
    scene.prepare_spatial_index();
    let again = cache
        .prepare(
            &scene.clone().camera(Camera::default()),
            1.,
            Some(ui),
            resolve,
        )
        .unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    let wider = cache.prepare(&scene, 2., Some(ui), resolve).unwrap();
    assert!(!Arc::ptr_eq(&first, &wider));
    assert_ne!(first.frame().view_projection, wider.frame().view_projection);
    let shifted = scene.clone().camera(Camera {
        lens_shift: [0.25, 0.],
        ..Default::default()
    });
    let moved = cache.prepare(&shifted, 2., Some(ui), resolve).unwrap();
    assert_ne!(wider.frame().view_projection, moved.frame().view_projection);
    let dense_ui = UiTexture3d::new(size(px(200.), px(100.)), 2.);
    let dense = cache
        .prepare(&shifted, 2., Some(dense_ui), resolve)
        .unwrap();
    assert!(!Arc::ptr_eq(&moved, &dense));
    assert_eq!(
        dense.frame().ui_texture.unwrap().pixel_size(),
        size(DevicePixels(400), DevicePixels(200))
    );
    let large_ui = UiTexture3d::new(size(px(400.), px(200.)), 1.);
    let large = cache
        .prepare(&shifted, 2., Some(large_ui), resolve)
        .unwrap();
    assert!(!Arc::ptr_eq(&dense, &large));
    assert_eq!(
        large.frame().ui_texture.unwrap().logical_size(),
        large_ui.logical_size()
    );
    let absent = cache.prepare(&shifted, 2., None, resolve).unwrap();
    assert!(absent.frame().ui_texture.is_none());
    cache.clear();
    let rebuilt = cache.prepare(&shifted, 2., None, resolve).unwrap();
    assert!(!Arc::ptr_eq(&absent, &rebuilt));
    assert_eq!(first.object(1).unwrap().id, Some("panel".into()));
    assert_eq!(first.frame().ui_texture.unwrap().scale_factor(), 1.);
}

#[test]
fn retained_resources_refresh_once_per_slot_and_preserve_older_outputs() {
    let scene = Scene::new()
        .object(
            Object::new(
                Mesh::plane(),
                layered_material().alpha_mode(gpui_3d::AlphaMode::Blend),
            )
            .position([0.1, 0.2, -0.3])
            .rotation([0.2, 0.3, 0.4])
            .scale([1.2, 0.8, 1.])
            .id("surface"),
        )
        .object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))).id("solid"))
        .object(
            Object::new(Mesh::plane(), Material::image("offscreen.png")).position([100., 0., 0.]),
        );
    let mut cache = PreparationCache::new();
    let mut previous: Option<Arc<gpui_3d::PreparedScene>> = None;
    let mut retained_pending = None;
    let mut retained_ready: Option<Arc<gpui_3d::PreparedScene>> = None;
    for (step, ready, tile_id) in [
        (0, false, 9),
        (1, false, 9),
        (2, true, 9),
        (3, true, 9),
        (4, true, 10),
        (5, false, 10),
    ] {
        let mut requests = Vec::new();
        let prepared = cache
            .prepare(&scene, 1., None, |request| {
                requests.push((request.object_index, request.slot));
                assert!(request.object_index < 2);
                if request.object_index == 1 {
                    Ok(TextureState::Ready(ResolvedTexture::None))
                } else {
                    assert_eq!(request.object_id, Some(&"surface".into()));
                    Ok(if ready {
                        TextureState::Ready(ResolvedTexture::Image(AtlasTile {
                            tile_id: TileId(tile_id),
                            ..tile()
                        }))
                    } else {
                        TextureState::Pending
                    })
                }
            })
            .unwrap();
        assert_eq!(requests.len(), 6);
        for slot in [
            TextureSlot::MetallicRoughness,
            TextureSlot::Emissive,
            TextureSlot::Normal,
            TextureSlot::Occlusion,
            TextureSlot::BaseColor,
        ] {
            assert_eq!(
                requests
                    .iter()
                    .filter(|&&(index, s)| index == 0 && s == slot)
                    .count(),
                1
            );
        }
        assert_eq!(prepared.is_ready(), ready);
        assert_eq!(prepared.frame().objects.len(), if ready { 2 } else { 1 });
        assert_eq!(prepared.objects().len(), 3);
        if ready {
            if let Some(first) = &retained_ready {
                let old = &first.frame().objects[0];
                let current = &prepared.frame().objects[0];
                assert!(Arc::ptr_eq(&old.mesh, &current.mesh));
                assert_eq!(old.model, current.model);
                assert_eq!(old.normal, current.normal);
                assert_eq!(old.sort_depth, current.sort_depth);
                assert_eq!(old.sampling, current.sampling);
                assert_eq!(old.uv_set, current.uv_set);
                assert_eq!(old.cast_shadows, current.cast_shadows);
                assert_eq!(old.receive_shadows, current.receive_shadows);
                assert_eq!(old.normal_texture.unwrap().tile.tile_id, TileId(9));
            } else {
                retained_ready = Some(prepared.clone());
            }
            assert_eq!(
                prepared.frame().objects[0]
                    .normal_texture
                    .unwrap()
                    .tile
                    .tile_id,
                TileId(tile_id)
            );
        } else {
            assert_eq!(prepared.frame().objects[0].output_id, 2);
        }
        if let Some(previous) = previous {
            assert!(Arc::ptr_eq(&previous.identities(), &prepared.identities()));
            assert_eq!(Arc::ptr_eq(&previous, &prepared), step == 1 || step == 3);
        } else {
            retained_pending = Some(prepared.clone());
        }
        previous = Some(prepared);
    }
    let pending = retained_pending.unwrap();
    assert_eq!(pending.pending_textures().len(), 5);
    assert_eq!(pending.frame().objects.len(), 1);
}

#[test]
fn invalid_later_objects_do_not_start_resource_resolution() {
    let scene = Scene::new()
        .object(Object::new(Mesh::plane(), Material::image("surface.png")))
        .object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))).scale([1., 0., 1.]));
    let mut requests = 0;
    let mut resolve = |_: gpui_3d::TextureRequest<'_>| {
        requests += 1;
        Ok(TextureState::Ready(ResolvedTexture::Image(tile())))
    };
    assert!(matches!(
        scene.prepare(1., None, &mut resolve),
        Err(PrepareError::InvalidScene(_))
    ));
    assert!(matches!(
        PreparationCache::new().prepare(&scene, 1., None, &mut resolve),
        Err(PrepareError::InvalidScene(_))
    ));
    assert_eq!(requests, 0);
}

#[test]
fn cache_failures_never_return_a_stale_frame() {
    let scene = Scene::new().object(Object::new(Mesh::plane(), Material::image("base.png")));
    let mut cache = PreparationCache::new();
    let resolve =
        |_: gpui_3d::TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::Image(tile())));
    let first = cache.prepare(&scene, 1., None, resolve).unwrap();
    assert!(matches!(
        cache.prepare(&scene, 1., None, |_| Err(anyhow::anyhow!(
            "resource unavailable"
        ))),
        Err(PrepareError::Resource {
            object_index: 0,
            slot: TextureSlot::BaseColor,
            ..
        })
    ));
    let recovered = cache.prepare(&scene, 1., None, resolve).unwrap();
    assert!(!Arc::ptr_eq(&first, &recovered));
    assert!(matches!(
        cache.prepare(&scene, 1., None, |_| Ok(TextureState::Ready(
            ResolvedTexture::None
        ))),
        Err(PrepareError::InvalidResolution {
            object_index: 0,
            slot: TextureSlot::BaseColor
        })
    ));
    cache.prepare(&scene, 1., None, resolve).unwrap();
    assert!(matches!(
        cache.prepare(&scene, 0., None, |_| panic!(
            "invalid aspect reached resolver"
        )),
        Err(PrepareError::InvalidScene(_))
    ));
    let invalid = scene.clone().camera(Camera {
        near: -1.,
        ..Default::default()
    });
    assert!(matches!(
        cache.prepare(&invalid, 1., None, |_| panic!(
            "invalid camera reached resolver"
        )),
        Err(PrepareError::InvalidScene(_))
    ));
    assert_eq!(first.frame().objects.len(), 1);
}

#[test]
fn evaluated_snapshot_clones_reuse_preparation_but_distinct_poses_do_not() {
    use gpui_3d::AffineTransform;
    let mut graph = SceneGraph::new();
    let node = graph
        .insert(
            None,
            Node::new()
                .id("mesh")
                .mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let evaluated = graph.evaluate().unwrap();
    let mut cache = PreparationCache::new();
    let resolve = |_: gpui_3d::TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::None));
    let first = cache
        .prepare(&evaluated.scene(Camera::default()), 1., None, resolve)
        .unwrap();
    let snapshot_clone = evaluated.clone();
    let cloned = cache
        .prepare(&snapshot_clone.scene(Camera::default()), 1., None, resolve)
        .unwrap();
    assert!(Arc::ptr_eq(&first, &cloned));
    let pose = graph
        .evaluate_with_transforms([(
            node,
            AffineTransform::from_translation([1., 0., 0.]).unwrap(),
        )])
        .unwrap();
    assert_eq!(evaluated.revision(), pose.revision());
    let moved = cache
        .prepare(&pose.scene(Camera::default()), 1., None, resolve)
        .unwrap();
    assert_ne!(
        first.frame().objects[0].model,
        moved.frame().objects[0].model
    );
    assert_eq!(moved.object(1).unwrap().node, Some(node));
    graph
        .set_material(node, Material::color(rgb(0xff0000)))
        .unwrap();
    let changed = cache
        .prepare(
            &graph.evaluate().unwrap().scene(Camera::default()),
            1.,
            None,
            resolve,
        )
        .unwrap();
    assert_ne!(
        first.frame().objects[0].color,
        changed.frame().objects[0].color
    );
    graph.set_visible(node, false).unwrap();
    let hidden = cache
        .prepare(
            &graph.evaluate().unwrap().scene(Camera::default()),
            1.,
            None,
            |_| panic!("hidden object reached resolver"),
        )
        .unwrap();
    assert!(hidden.frame().objects.is_empty());
}

#[test]
fn scene_builders_invalidate_retained_render_inputs() {
    use gpui_3d::{
        ColorOutput, DiffuseEnvironment, DirectionalShadow, EnvironmentBackground, EnvironmentMap,
        Light, PunctualLight, SpecularEnvironment, SpecularEnvironmentMap,
    };
    let map = EnvironmentMap::from_equirectangular([1, 1], vec![[1.; 3]]).unwrap();
    let reflections = SpecularEnvironment::from_prefiltered(
        SpecularEnvironmentMap::from_prefiltered(1, vec![vec![[1.; 3]; 6]]).unwrap(),
    );
    let scene = Scene::new().object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))));
    let cases: [(Scene, fn(&gpui::Scene3dFrame)); 8] = [
        (
            scene.clone().light(Light {
                intensity: 2.,
                ..Default::default()
            }),
            |frame| assert_eq!(frame.light[3], 2.),
        ),
        (
            scene.clone().lights([PunctualLight::point([1., 1., 2.])]),
            |frame| assert_eq!(frame.lights.as_ref().unwrap().len(), 1),
        ),
        (
            scene
                .clone()
                .directional_shadow(Some(DirectionalShadow::new([0.; 3], [4.; 3]))),
            |frame| assert!(frame.directional_shadow.is_some()),
        ),
        (
            scene
                .clone()
                .diffuse_environment(DiffuseEnvironment::from_map(&map).unwrap()),
            |frame| assert!(frame.diffuse_environment.is_some()),
        ),
        (
            scene
                .clone()
                .background(Some(EnvironmentBackground::new(map))),
            |frame| assert!(frame.background.is_some()),
        ),
        (
            scene.clone().specular_environment(Some(reflections)),
            |frame| assert!(frame.specular_environment.is_some()),
        ),
        (
            scene.clone().color_output(ColorOutput {
                exposure: 2.,
                ..Default::default()
            }),
            |frame| assert_eq!(frame.color_output.exposure, 2.),
        ),
        (
            scene
                .clone()
                .object(Object::new(Mesh::plane(), Material::color(rgb(0xff0000))).id("extra")),
            |frame| assert_eq!(frame.objects.len(), 2),
        ),
    ];
    let resolve = |_: gpui_3d::TextureRequest<'_>| Ok(TextureState::Ready(ResolvedTexture::None));
    let mut cache = PreparationCache::new();
    for (changed, check) in cases {
        let original = cache.prepare(&scene, 1., None, resolve).unwrap();
        let updated = cache.prepare(&changed, 1., None, resolve).unwrap();
        assert!(!Arc::ptr_eq(&original, &updated));
        check(updated.frame());
        let reused = cache.prepare(&changed, 1., None, resolve).unwrap();
        assert!(Arc::ptr_eq(&updated, &reused));
    }
    let original = cache.prepare(&scene, 1., None, resolve).unwrap();
    let scene = scene.object(Object::new(Mesh::plane(), Material::color(rgb(0x00ff00))));
    let extended = cache.prepare(&scene, 1., None, resolve).unwrap();
    assert_eq!(original.frame().objects.len(), 1);
    assert_eq!(extended.frame().objects.len(), 2);
}
