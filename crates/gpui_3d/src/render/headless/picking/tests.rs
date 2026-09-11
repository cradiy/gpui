use super::*;
use crate::{DepthBackground, Material, Mesh, Node, Projection, SceneGraph};

fn pixels(id: u32, depth: f32, background: DepthBackground) -> Scene3dPixels {
    Scene3dPixels {
        depth_background: background,
        size: [1, 1],
        rgba: None,
        linear_rgba: None,
        object_ids: Some(vec![id]),
        linear_depth: Some(vec![depth]),
        world_normals: None,
    }
}

fn object() -> RenderObject {
    RenderObject {
        output_id: 1,
        object_index: 0,
        id: Some("surface".into()),
        node: None,
    }
}

#[test]
fn pick_reconstruction_uses_original_extent_pixel_center_and_camera_optics() {
    for projection in [
        Projection::Perspective { vertical_fov: 1.1 },
        Projection::Orthographic { vertical_size: 4. },
    ] {
        let camera = Camera {
            eye: [2., 3., 5.],
            target: [1., 2., 0.],
            lens_shift: [0.25, -0.4],
            aspect_ratio: Some(1.5),
            projection,
            ..Default::default()
        };
        let extent = [71, 39];
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(71.), px(39.)));
        for pixel in [[0, 0], [63, 7], [35, 19], [70, 38]] {
            let result = resolve(
                camera,
                extent,
                pixel,
                &[object()],
                &pixels(1, 4., DepthBackground::Zero),
            )
            .unwrap();
            let hit = result.hit.as_ref().unwrap();
            assert_eq!(result.pixel, pixel);
            assert_eq!(result.size, extent);
            assert_eq!(hit.object.id, object().id);
            assert_eq!(hit.linear_depth, 4.);
            let projected = camera
                .world_to_screen(bounds, hit.world_position)
                .unwrap()
                .unwrap();
            for (actual, expected) in [
                f32::from(projected.position.x),
                f32::from(projected.position.y),
            ]
            .into_iter()
            .zip(pixel.map(|v| v as f32 + 0.5))
            {
                assert!((actual - expected).abs() < 1e-4, "{actual} != {expected}");
            }
            assert!((projected.depth - 4.).abs() < 1e-5);
            assert_eq!(result.camera().lens_shift, camera.lens_shift);
        }
    }
}

#[test]
fn background_and_zero_depth_surfaces_have_distinct_pick_results() {
    let camera = Camera {
        eye: [0., 0., 5.],
        target: [0., 0., 0.],
        near: 0.,
        projection: Projection::Orthographic { vertical_size: 2. },
        ..Default::default()
    };
    for background in [DepthBackground::Zero, DepthBackground::NegativeOne] {
        let miss = resolve(
            camera,
            [3, 3],
            [1, 1],
            &[object()],
            &pixels(0, background.value(), background),
        )
        .unwrap();
        assert!(miss.hit.is_none());
        let surface = resolve(
            camera,
            [3, 3],
            [1, 1],
            &[object()],
            &pixels(1, 0., background),
        )
        .unwrap();
        assert_eq!(surface.hit.unwrap().world_position, [0., 0., 5.]);
        for invalid in [f32::NAN, f32::INFINITY, -2.] {
            assert!(
                resolve(
                    camera,
                    [3, 3],
                    [1, 1],
                    &[object()],
                    &pixels(1, invalid, background)
                )
                .is_err()
            );
        }
        assert!(
            resolve(
                camera,
                [3, 3],
                [1, 1],
                &[object()],
                &pixels(0, 4., background)
            )
            .is_err()
        );
    }
    assert!(
        resolve(
            Camera::default(),
            [3, 3],
            [1, 1],
            &[object()],
            &pixels(1, 0., DepthBackground::Zero)
        )
        .is_err()
    );
}

#[test]
fn pick_identity_remains_frame_local_and_malformed_samples_are_errors() {
    let mut graph = SceneGraph::new();
    let node = graph
        .insert(
            None,
            Node::new()
                .id("original")
                .mesh(Mesh::cube(), Material::color(gpui::white())),
        )
        .unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    let prepared = scene
        .prepare(1., None, |_| {
            Ok(crate::TextureState::Ready(crate::ResolvedTexture::None))
        })
        .unwrap();
    let objects = prepared.identities();
    graph.remove_subtree(node).unwrap();
    graph
        .insert(
            None,
            Node::new()
                .id("replacement")
                .mesh(Mesh::cube(), Material::color(gpui::white())),
        )
        .unwrap();
    let sample = pixels(1, 4., DepthBackground::Zero);
    let result = resolve(Camera::default(), [9, 7], [4, 3], &objects, &sample).unwrap();
    drop(prepared);
    drop(objects);
    drop(graph);
    let hit = result.hit.unwrap();
    assert_eq!(hit.object.node, Some(node));
    assert_eq!(hit.object.id, Some("original".into()));

    for id in [2, u32::MAX] {
        assert!(
            resolve(
                Camera::default(),
                [9, 7],
                [4, 3],
                &[object()],
                &pixels(id, 4., DepthBackground::Zero)
            )
            .is_err()
        );
    }
    for pixel in [[9, 0], [0, 7], [u32::MAX; 2]] {
        assert!(resolve(Camera::default(), [9, 7], pixel, &[object()], &sample).is_err());
    }
    for size in [[0, 7], [9, 0]] {
        assert!(resolve(Camera::default(), size, [0, 0], &[object()], &sample).is_err());
    }
    let mut invalid = sample;
    invalid.size = [2, 1];
    assert!(resolve(Camera::default(), [9, 7], [0, 0], &[object()], &invalid).is_err());
    invalid.size = [1, 1];
    for ids in [None, Some(vec![]), Some(vec![1, 2])] {
        invalid.object_ids = ids;
        assert!(resolve(Camera::default(), [9, 7], [0, 0], &[object()], &invalid).is_err());
    }
    invalid.object_ids = Some(vec![1]);
    for depths in [None, Some(vec![]), Some(vec![4., 4.])] {
        invalid.linear_depth = depths;
        assert!(resolve(Camera::default(), [9, 7], [0, 0], &[object()], &invalid).is_err());
    }
}
