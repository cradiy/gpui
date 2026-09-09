use gpui::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, TileId, UiTexture3d, point,
    px, rgb, size,
};
use gpui_3d::{
    Camera, Material, MaterialTexture, Mesh, Node, Object, PbrMaterial, PrepareError,
    ResolvedTexture, Scene, SceneGraph, TextureSlot, TextureSource, TextureState,
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

fn layered_material() -> Material {
    Material::image("base.png")
        .pbr(PbrMaterial::default())
        .metallic_roughness_texture(MaterialTexture::new("surface.png"))
        .emissive_texture(MaterialTexture::new("emission.png"))
        .normal_texture(MaterialTexture::new("normal.png"))
        .occlusion_texture(MaterialTexture::new("occlusion.png"))
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
