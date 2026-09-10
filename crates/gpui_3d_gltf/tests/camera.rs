use gpui::{Bounds, point, px, size};
use gpui_3d::{Camera, Projection, ResolvedTexture, SceneGraph, TextureState};
use gpui_3d_gltf::{Document, ImageDecodeLimits, Limits, PreparedDocument, SceneOptions};
use serde_json::{Value, json};

fn document(cameras: Value) -> PreparedDocument {
    let source = json!({"asset":{"version":"2.0"},"cameras":cameras});
    Document::from_slice(&serde_json::to_vec(&source).unwrap(), Limits::default())
        .unwrap()
        .prepare(|_| panic!("no resources"))
        .unwrap()
}

#[test]
fn perspective_conversion_preserves_fov_fixed_aspect_and_optional_infinite_far() {
    let document = document(json!([
        {"type":"perspective","perspective":{"yfov":1.2,"aspectRatio":2.,"znear":0.25,"zfar":50.}},
        {"type":"perspective","perspective":{"yfov":1.2,"znear":0.25}}
    ]));
    let fixed = document.camera(0).unwrap();
    let adaptive = document.camera(1).unwrap();
    assert_eq!((fixed.near, fixed.far), (0.25, 50.));
    assert_eq!((adaptive.near, adaptive.far), (0.25, f32::INFINITY));
    for aspect in [0.5, 1., 3.] {
        let viewport = Bounds::new(point(px(20.), px(30.)), size(px(400. * aspect), px(400.)));
        let half_height = 0.6_f32.tan() * 4.;
        let right = fixed
            .world_to_screen(viewport, [2. * half_height, 0., -4.])
            .unwrap()
            .unwrap();
        assert!((f32::from(right.position.x - viewport.right())).abs() < 1e-3);
        let right = adaptive
            .world_to_screen(viewport, [aspect * half_height, 0., -4.])
            .unwrap()
            .unwrap();
        assert!((f32::from(right.position.x - viewport.right())).abs() < 1e-3);
        assert!(
            adaptive
                .world_to_screen(viewport, [0., 0., -10000.])
                .unwrap()
                .unwrap()
                .in_frustum
        );
        assert!(
            !fixed
                .world_to_screen(viewport, [0., 0., -10000.])
                .unwrap()
                .unwrap()
                .in_frustum
        );
    }
    assert!(document.camera(2).is_err());
}

#[test]
fn orthographic_conversion_preserves_both_magnitudes_independent_of_output_shape() {
    let document = document(
        json!([{"type":"orthographic","orthographic":{"xmag":3.,"ymag":2.,"znear":0.5,"zfar":30.}}]),
    );
    let camera = document.camera(0).unwrap();
    assert_eq!(
        camera.projection,
        Projection::Orthographic { vertical_size: 4. }
    );
    for (width, height) in [(300., 400.), (600., 400.)] {
        let viewport = Bounds::new(point(px(0.), px(0.)), size(px(width), px(height)));
        for depth in [1., 20.] {
            let corner = camera
                .world_to_screen(viewport, [3., 2., -depth])
                .unwrap()
                .unwrap();
            assert!((f32::from(corner.position.x) - width).abs() < 1e-4);
            assert!(f32::from(corner.position.y).abs() < 1e-4);
        }
    }
}

#[test]
fn zero_near_orthographic_conversion_keeps_the_authored_camera_plane() {
    let source = document(
        json!([{"type":"orthographic","orthographic":{"xmag":3.,"ymag":2.,"znear":0.,"zfar":30.}}]),
    );
    let camera = source.camera(0).unwrap();
    assert_eq!(camera.near, 0.);
    let viewport = Bounds::new(point(px(20.), px(30.)), size(px(600.), px(400.)));
    let point = camera
        .world_to_screen(viewport, [3., 2., 0.])
        .unwrap()
        .unwrap();
    assert!(point.in_frustum);
    assert_eq!(point.depth, 0.);
    assert_eq!(
        camera
            .screen_to_world(viewport, point.position, point.depth)
            .unwrap(),
        [3., 2., 0.]
    );
}

#[test]
fn camera_nodes_keep_parent_pose_source_indices_and_independent_instance_selection() {
    let source = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"translation":[1,0,0],"rotation":[0,std::f32::consts::FRAC_1_SQRT_2,0,std::f32::consts::FRAC_1_SQRT_2],"children":[1,2]},
            {"name":"view","translation":[0,0,2],"camera":0}, {"translation":[0,1,2],"camera":0}],
        "cameras":[{"type":"perspective","perspective":{"yfov":1.,"znear":0.1,"aspectRatio":1.5}}]});
    let document = Document::from_slice(&serde_json::to_vec(&source).unwrap(), Limits::default())
        .unwrap()
        .prepare(|_| panic!())
        .unwrap();
    let definition = document.scene(None, SceneOptions::default()).unwrap();
    let asset = definition
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    assert_eq!(
        asset
            .nodes()
            .iter()
            .map(|node| node.camera_index)
            .collect::<Vec<_>>(),
        [None, Some(0), Some(0)]
    );
    assert!(asset.primitives().is_empty());
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree()).unwrap();
    let second = graph.instantiate(None, asset.subtree()).unwrap();
    let first_camera = first.node(asset.nodes()[1].handle).unwrap();
    let second_camera = second.node(asset.nodes()[1].handle).unwrap();
    let evaluated = graph.evaluate().unwrap();
    let camera = evaluated.node(first_camera).unwrap().camera.unwrap();
    for (actual, expected) in camera.eye.into_iter().zip([3., 0., 0.]) {
        assert!((actual - expected).abs() < 1e-5);
    }
    for (actual, expected) in camera.target.into_iter().zip([2., 0., 0.]) {
        assert!((actual - expected).abs() < 1e-5);
    }
    let scene = evaluated.scene_from_camera(first_camera).unwrap();
    let frame = scene
        .prepare(2., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert!(frame.frame().objects.is_empty());
    assert_eq!(
        frame.frame().view_projection,
        camera.view_projection(2.).unwrap()
    );
    graph
        .set_camera(
            second_camera,
            Some(Camera {
                aspect_ratio: Some(1.),
                ..document.camera(0).unwrap()
            }),
        )
        .unwrap();
    let updated = graph.evaluate().unwrap();
    assert_eq!(
        updated
            .node(first_camera)
            .unwrap()
            .camera
            .unwrap()
            .aspect_ratio,
        Some(1.5)
    );
    assert_eq!(
        updated
            .node(second_camera)
            .unwrap()
            .camera
            .unwrap()
            .aspect_ratio,
        Some(1.)
    );
}

#[test]
fn mismatched_camera_schema_and_invalid_projection_parameters_fail_without_panics() {
    for camera in [
        json!({"type":"perspective","orthographic":{"xmag":1.,"ymag":1.,"znear":0.1,"zfar":10.}}),
        json!({"type":"orthographic","perspective":{"yfov":1.,"znear":0.1}}),
        json!({"type":"perspective","perspective":{"yfov":1.,"znear":0.1},"orthographic":{"xmag":1.,"ymag":1.,"znear":0.1,"zfar":10.}}),
    ] {
        let bytes =
            serde_json::to_vec(&json!({"asset":{"version":"2.0"},"cameras":[camera]})).unwrap();
        let error = Document::from_slice(&bytes, Limits::default())
            .err()
            .unwrap();
        assert!(format!("{error:#}").contains("camera 0"));
    }
    for camera in [
        json!({"type":"perspective","perspective":{"yfov":0.,"znear":0.1}}),
        json!({"type":"perspective","perspective":{"yfov":1.,"znear":0.1,"zfar":0.01}}),
        json!({"type":"perspective","perspective":{"yfov":1.,"znear":0.1,"aspectRatio":0.}}),
        json!({"type":"perspective","perspective":{"yfov":1.,"znear":0.1,"zfar":1e40}}),
        json!({"type":"orthographic","orthographic":{"xmag":1.,"ymag":1.,"znear":-0.1,"zfar":10.}}),
        json!({"type":"orthographic","orthographic":{"xmag":-1.,"ymag":1.,"znear":0.1,"zfar":10.}}),
    ] {
        let error = document(json!([camera])).camera(0).unwrap_err();
        assert!(format!("{error:#}").contains("camera 0"));
    }
}
