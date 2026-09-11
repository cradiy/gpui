use super::*;
use crate::{DepthRelation, Projection};

fn frame(projection: Projection) -> ReadFrame {
    ReadFrame {
        layout: FrameReadbackLayout {
            output_size: [20, 12],
            region: Scene3dReadbackRegion {
                origin: [7, 5],
                size: [3, 2],
            },
            projection_rect: [-2.5, -1.25, 25., 16.],
        },
        frame_id: Default::default(),
        camera: Camera {
            projection,
            aspect_ratio: Some(1.3),
            lens_shift: [0.2, -0.3],
            ..Default::default()
        },
        objects: (1..=2)
            .map(|id| RenderObject {
                output_id: id,
                object_index: id as usize - 1,
                id: Some(format!("part-{id}").into()),
                node: None,
            })
            .collect(),
        pixels: Scene3dPixels {
            size: [3, 2],
            depth_background: Default::default(),
            rgba: None,
            linear_rgba: None,
            object_ids: Some(vec![0, 1, 1, 1, 1, 1]),
            linear_depth: Some(vec![0., 4., 4., 4., 4., 4.]),
            world_normals: None,
        },
    }
}

#[test]
fn regional_world_and_depth_queries_preserve_projection_and_absolute_pixels() {
    for projection in [
        Projection::Perspective { vertical_fov: 1.1 },
        Projection::Orthographic { vertical_size: 4. },
    ] {
        let frame = frame(projection);
        let viewport = Bounds::new(point(px(-2.5), px(-1.25)), size(px(25.), px(16.)));
        assert!(frame.world_position_at(0, 0).unwrap().is_none());
        assert!(frame.world_position_at(3, 0).unwrap().is_none());
        for (x, y) in [(1, 0), (2, 0), (0, 1), (1, 1), (2, 1)] {
            let world = frame.world_position_at(x, y).unwrap().unwrap();
            let projected = frame
                .camera
                .world_to_screen(viewport, world)
                .unwrap()
                .unwrap();
            assert!((f32::from(projected.position.x) - (7 + x) as f32 - 0.5).abs() < 1e-4);
            assert!((f32::from(projected.position.y) - (5 + y) as f32 - 0.5).abs() < 1e-4);
            let compared = frame.compare_depth(world, 1e-4).unwrap().unwrap();
            assert_eq!(compared.pixel, [7 + x, 5 + y]);
            assert_eq!(compared.relation, DepthRelation::WithinTolerance);
            assert_eq!(frame.object_at(x, y).unwrap().output_id, 1);
        }
        let outside = frame
            .camera
            .screen_to_world(viewport, point(px(6.5), px(5.5)), 4.)
            .unwrap();
        assert!(frame.compare_depth(outside, 0.).unwrap().is_none());
        let right = frame
            .camera
            .screen_to_world(viewport, point(px(10.5), px(5.5)), 4.)
            .unwrap();
        assert!(frame.compare_depth(right, 0.).unwrap().is_none());
    }
}

#[test]
fn regional_coverage_and_labels_keep_source_extent_and_unobserved_objects() {
    let frame = frame(Projection::Perspective { vertical_fov: 1. });
    let identity = frame.frame_id().clone();
    let layout = frame.layout();
    let coverage = frame.coverage().unwrap();
    let labels = frame
        .label_image(6, |object| object.output_id + 20)
        .unwrap();
    drop(frame);
    assert_eq!(coverage.layout(), layout);
    assert_eq!(labels.layout(), layout);
    assert_eq!(coverage.frame_id(), &identity);
    assert_eq!(labels.frame_id(), &identity);
    assert_eq!(coverage.pixel_count(), 6);
    assert_eq!(coverage.background_pixels(), 1);
    let visible = coverage.object(1).unwrap();
    assert_eq!(visible.pixels, 5);
    assert_eq!(visible.screen_fraction, 5. / 240.);
    assert_eq!(visible.bounds, Some(Bounds::new(point(0, 0), size(3, 2))));
    assert_eq!(coverage.object(2).unwrap().pixels, 0);
    assert_eq!(labels.pixels(), &[0, 21, 21, 21, 21, 21]);
    assert_eq!(labels.label_for_object(2), Some(22));
}

#[test]
fn malformed_region_metadata_is_rejected_before_coordinate_arithmetic() {
    let mut frame = frame(Projection::Perspective { vertical_fov: 1. });
    let original = frame.layout;
    for layout in [
        FrameReadbackLayout {
            region: Scene3dReadbackRegion {
                origin: [u32::MAX, 0],
                size: [3, 2],
            },
            ..original
        },
        FrameReadbackLayout {
            output_size: [8, 6],
            ..original
        },
        FrameReadbackLayout {
            projection_rect: [0., 0., f32::NAN, 2.],
            ..original
        },
        FrameReadbackLayout {
            region: Scene3dReadbackRegion {
                origin: [0; 2],
                size: [2, 3],
            },
            ..original
        },
    ] {
        frame.layout = layout;
        assert!(frame.world_position_at(1, 0).is_err());
        assert!(frame.compare_depth([0.; 3], 0.).is_err());
        assert!(frame.coverage().is_err());
        assert!(frame.label_image(6, |_| 1).is_err());
        assert!(frame.object_at(1, 0).is_none());
    }
}
