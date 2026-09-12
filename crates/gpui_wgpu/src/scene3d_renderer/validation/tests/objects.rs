use super::*;
use crate::wgpu_renderer::scene3d::tests::object;
use gpui::{MaterialTexture3d, MeshDraw3d, MeshTexture3d};

fn map(set: u32) -> MaterialTexture3d {
    MaterialTexture3d {
        tile: gpui::AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 0,
                kind: gpui::AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(0),
            padding: 0,
            bounds: gpui::Bounds::new(
                gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                gpui::size(gpui::DevicePixels(1), gpui::DevicePixels(1)),
            ),
        },
        uv_set: set,
        sampling: Default::default(),
    }
}

#[test]
fn active_texture_coordinates_and_tangent_basis_follow_mesh_snapshots() {
    let mut source = object();
    source.uv_set = 7;
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_ok());
    source.texture = MeshTexture3d::Image(map(7).tile);
    let error = validate_frame_settings(&frame(&[source.clone()]), 1024, true).unwrap_err();
    assert!(error.to_string().contains("missing UV set 7"));
    source.mesh = source.mesh.with_uv_set(7, vec![[0.; 2]; 3]).unwrap();
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_ok());

    source.pbr = Some(Default::default());
    source.unlit = false;
    source.normal_texture = Some(map(7));
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_err());
    source.mesh = source
        .mesh
        .with_tangents_for_uv_set(0, vec![[1., 0., 0., 1.]; 3])
        .unwrap();
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_err());
    source.mesh = source
        .mesh
        .with_tangents_for_uv_set(7, vec![[1., 0., 0., 1.]; 3])
        .unwrap();
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_ok());
    source.mesh = source.mesh.with_uv_set(7, vec![[0.5; 2]; 3]).unwrap();
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_err());
    source.normal_scale = 0.;
    assert!(validate_frame_settings(&frame(&[source]), 1024, true).is_ok());
}

#[test]
fn invalid_sampling_is_rejected_only_for_active_texture_slots() {
    let mut source = object();
    source.sampling.max_anisotropy = 0;
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_ok());
    source.texture = MeshTexture3d::Image(map(0).tile);
    assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_err());
    source.sampling = Default::default();
    source.pbr = Some(Default::default());
    source.unlit = false;
    let slots: &[fn(&mut MeshDraw3d, MaterialTexture3d)] = &[
        |object, map| object.metallic_roughness_texture = Some(map),
        |object, map| object.emissive_texture = Some(map),
        |object, map| object.normal_texture = Some(map),
        |object, map| object.occlusion_texture = Some(map),
    ];
    for slot in slots {
        let mut object = source.clone();
        let mut texture = map(0);
        texture.sampling.max_anisotropy = 2;
        slot(&mut object, texture);
        assert!(validate_frame_settings(&frame(&[object.clone()]), 1024, true).is_err());
        object.unlit = true;
        assert!(validate_frame_settings(&frame(&[object]), 1024, true).is_ok());
    }
}

#[test]
fn invalid_object_parameters_are_rejected_even_outside_the_camera() {
    let edits: &[fn(&mut MeshDraw3d)] = &[
        |object| object.render_bounds = Some([[1.; 3], [-1.; 3]]),
        |object| object.render_bounds = Some([[f32::NAN; 3], [1.; 3]]),
        |object| object.model[0][0] = f32::NAN,
        |object| object.normal[2][1] = f32::INFINITY,
        |object| object.color.a = f32::NAN,
        |object| object.alpha_cutoff = -1.,
        |object| object.normal_scale = -1.,
        |object| object.occlusion_strength = 2.,
        |object| object.sort_depth = f64::NAN,
        |object| {
            object.pbr = Some(gpui::PbrMaterial3d {
                roughness: -1.,
                ..Default::default()
            })
        },
    ];
    for edit in edits {
        let mut source = object();
        source.model[3][0] = 1000.;
        assert!(validate_frame_settings(&frame(&[source.clone()]), 1024, true).is_ok());
        edit(&mut source);
        assert!(validate_frame_settings(&frame(&[source]), 1024, true).is_err());
    }
}
