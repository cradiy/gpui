use super::*;
use crate::{Camera, Material, Mesh, Node, Ray, SceneError, SceneGraph};
use gpui::rgb;

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}

fn close<const N: usize>(actual: [f32; N], expected: [f32; N]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 2e-5, "{actual} != {expected}");
    }
}

#[test]
fn vector_tracks_clamp_hold_and_seek_without_history() {
    let keys = [
        Keyframe::new(seconds(2.), [1., 2., 3.]),
        Keyframe::new(seconds(4.), [5., -2., 7.]),
        Keyframe::new(seconds(7.), [-1., 4., 1.]),
    ];
    let linear = VectorTrack::new(keys, Interpolation::Linear).unwrap();
    let step = VectorTrack::new(keys, Interpolation::Step).unwrap();
    for (time, expected) in [
        (9., [-1., 4., 1.]),
        (3., [3., 0., 5.]),
        (0., [1., 2., 3.]),
        (5.5, [2., 1., 4.]),
        (4., [5., -2., 7.]),
        (3., [3., 0., 5.]),
    ] {
        close(linear.sample(seconds(time)).unwrap(), expected);
    }
    close(step.sample(seconds(3.999)).unwrap(), keys[0].value);
    close(step.sample(seconds(4.)).unwrap(), keys[1].value);
    close(step.sample(seconds(6.)).unwrap(), keys[1].value);
    for mode in [
        Interpolation::Step,
        Interpolation::Linear,
        Interpolation::CubicSpline,
    ] {
        let constant = VectorTrack::new([keys[1]], mode).unwrap();
        for time in [Duration::ZERO, Duration::MAX] {
            close(constant.sample(time).unwrap(), keys[1].value);
        }
    }
}

#[test]
fn cubic_derivatives_use_segment_seconds_and_exact_keys() {
    // x(t) = t^3 over [2, 5]; y(t) = 2t; z(t) = -t^2.
    let curve = VectorTrack::new(
        [
            Keyframe::new(seconds(2.), [8., 4., -4.]).tangents([0.; 3], [12., 2., -4.]),
            Keyframe::new(seconds(5.), [125., 10., -25.]).tangents([75., 2., -10.], [0.; 3]),
        ],
        Interpolation::CubicSpline,
    )
    .unwrap();
    for t in [2.5_f64, 4., 3., 5., 2.] {
        close(
            curve.sample(seconds(t)).unwrap(),
            [t.powi(3) as f32, (2. * t) as f32, -t.powi(2) as f32],
        );
    }
}

#[test]
fn sampling_preserves_nanosecond_offsets_at_large_absolute_times() {
    let start = Duration::from_secs(u64::MAX - 1);
    let curve = VectorTrack::new(
        [
            Keyframe::new(start, [-f32::MAX, 2., 4.]),
            Keyframe::new(start + Duration::from_nanos(4), [f32::MAX, 6., -4.]),
        ],
        Interpolation::Linear,
    )
    .unwrap();
    close(
        curve.sample(start + Duration::from_nanos(2)).unwrap(),
        [0., 4., 0.],
    );
    close(
        curve.sample(start + Duration::from_nanos(3)).unwrap()[1..]
            .try_into()
            .unwrap(),
        [5., -2.],
    );
}

#[test]
fn rotation_uses_shortest_arc_and_constant_angular_speed() {
    let rotation = RotationTrack::new(
        [
            Keyframe::new(Duration::ZERO, [0., 0., 0., 2.]),
            Keyframe::new(seconds(4.), [0., 0., -(3_f32).sqrt(), -1.]),
        ],
        Interpolation::Linear,
    )
    .unwrap();
    for (time, angle) in [(1., 30_f32), (3., 90.), (2., 60.)] {
        let transform =
            AffineTransform::from_trs([0.; 3], rotation.sample(seconds(time)).unwrap(), [1.; 3])
                .unwrap();
        close(
            transform.transform_point([1., 0., 0.]),
            [angle.to_radians().cos(), angle.to_radians().sin(), 0.],
        );
    }
    let antipodal = RotationTrack::new(
        [
            Keyframe::new(Duration::ZERO, [0., f32::MIN_POSITIVE, 0., 0.]),
            Keyframe::new(seconds(1.), [0., -f32::MAX, 0., 0.]),
        ],
        Interpolation::Linear,
    )
    .unwrap();
    close(antipodal.sample(seconds(0.5)).unwrap(), [0., 1., 0., 0.]);
}

#[test]
fn cubic_rotation_retains_component_tangents_and_reports_zero_crossings() {
    let curve = RotationTrack::new(
        [
            Keyframe::new(Duration::ZERO, [0., 0., 0., 1.]).tangents([0.; 4], [0., 0., 1., 0.]),
            Keyframe::new(seconds(2.), [0., 0., 1., 0.]),
        ],
        Interpolation::CubicSpline,
    )
    .unwrap();
    // Midpoint raw components are [0, 0, 0.75, 0.5].
    close(
        curve.sample(seconds(1.)).unwrap(),
        [0., 0., 3. / 13_f32.sqrt(), 2. / 13_f32.sqrt()],
    );
    let crossing = RotationTrack::new(
        [
            Keyframe::new(Duration::ZERO, [0., 0., 0., 1.]),
            Keyframe::new(seconds(2.), [0., 0., 0., -1.]),
        ],
        Interpolation::CubicSpline,
    )
    .unwrap();
    assert_eq!(
        crossing.sample(seconds(1.)),
        Err(AnimationError::InvalidSample)
    );
    close(crossing.sample(seconds(2.)).unwrap(), [0., 0., 0., -1.]);
}

#[test]
fn malformed_tracks_and_unrepresentable_samples_are_rejected() {
    assert!(matches!(
        VectorTrack::new([], Interpolation::Linear),
        Err(AnimationError::EmptyTrack)
    ));
    for times in [[1., 1.], [2., 1.]] {
        assert!(matches!(
            VectorTrack::new(
                times.map(|t| Keyframe::new(seconds(t), [0.; 3])),
                Interpolation::Linear
            ),
            Err(AnimationError::NonIncreasingTime { key: 1 })
        ));
    }
    assert!(matches!(
        VectorTrack::new(
            [Keyframe::new(Duration::ZERO, [f32::NAN, 0., 0.])],
            Interpolation::Step
        ),
        Err(AnimationError::NonFiniteKey { key: 0 })
    ));
    assert!(matches!(
        RotationTrack::new(
            [Keyframe::new(Duration::ZERO, [0.; 4])],
            Interpolation::Step
        ),
        Err(AnimationError::InvalidRotation { key: 0 })
    ));
    assert!(matches!(
        VectorTrack::new(
            [Keyframe::new(Duration::ZERO, [0.; 3]).tangents([f32::INFINITY; 3], [0.; 3])],
            Interpolation::CubicSpline
        ),
        Err(AnimationError::NonFiniteKey { key: 0 })
    ));
    let overflow = VectorTrack::new(
        [
            Keyframe::new(Duration::ZERO, [0.; 3]).tangents([0.; 3], [f32::MAX; 3]),
            Keyframe::new(seconds(16.), [0.; 3]),
        ],
        Interpolation::CubicSpline,
    )
    .unwrap();
    assert_eq!(
        overflow.sample(seconds(8.)),
        Err(AnimationError::InvalidSample)
    );
    close(overflow.sample(seconds(16.)).unwrap(), [0.; 3]);
}

#[test]
fn transform_channels_preserve_base_pose_and_reject_singular_scale() {
    let base = TransformPose {
        translation: [2., 3., 4.],
        scale: [-2., 3., 1.],
        ..Default::default()
    };
    let track = TransformTrack::new(base).unwrap().rotation(
        RotationTrack::new(
            [
                Keyframe::new(Duration::ZERO, [0., 0., 0., 1.]),
                Keyframe::new(seconds(2.), [0., 0., 1., 0.]),
            ],
            Interpolation::Linear,
        )
        .unwrap(),
    );
    let pose = track.sample(seconds(1.)).unwrap();
    assert_eq!(pose.translation, base.translation);
    assert_eq!(pose.scale, base.scale);
    let transform = pose.affine().unwrap();
    close(transform.transform_point([1., 0., 0.]), [2., 1., 4.]);
    close(
        transform.inverse().transform_point([2., 1., 4.]),
        [1., 0., 0.],
    );
    let crossing = track.scale(
        VectorTrack::new(
            [
                Keyframe::new(Duration::ZERO, [1.; 3]),
                Keyframe::new(seconds(2.), [-1., 1., 1.]),
            ],
            Interpolation::Linear,
        )
        .unwrap(),
    );
    assert!(matches!(
        crossing.sample(seconds(1.)),
        Err(AnimationError::InvalidTransform(_))
    ));
    assert!(crossing.sample(seconds(2.)).is_ok());
}

#[test]
fn independent_channel_ranges_compose_translation_rotation_and_scale() {
    let track = TransformTrack::default()
        .translation(
            VectorTrack::new(
                [
                    Keyframe::new(seconds(2.), [0.; 3]),
                    Keyframe::new(seconds(4.), [4., 0., 0.]),
                ],
                Interpolation::Linear,
            )
            .unwrap(),
        )
        .rotation(
            RotationTrack::new(
                [
                    Keyframe::new(Duration::ZERO, [0., 0., 0., 1.]),
                    Keyframe::new(seconds(2.), [0., 0., 1., 0.]),
                ],
                Interpolation::Linear,
            )
            .unwrap(),
        )
        .scale(
            VectorTrack::new(
                [
                    Keyframe::new(seconds(1.), [1.; 3]),
                    Keyframe::new(seconds(5.), [3., 5., 1.]),
                ],
                Interpolation::Linear,
            )
            .unwrap(),
        );
    let sampled = track.sample_transform(seconds(3.)).unwrap();
    close(sampled.transform_point([1.; 3]), [0., -3., 1.]);
    close(sampled.inverse().transform_point([0., -3., 1.]), [1.; 3]);
}

#[test]
fn evaluated_poses_update_hierarchy_bounds_and_picking_without_mutation() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let child = graph
        .insert(
            Some(root),
            Node::new()
                .id("surface")
                .mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let authored = graph.evaluate().unwrap();
    authored.prepare_spatial_index();
    let track = TransformTrack::default().translation(
        VectorTrack::new(
            [
                Keyframe::new(Duration::ZERO, [0.; 3]),
                Keyframe::new(seconds(2.), [4., 0., 0.]),
            ],
            Interpolation::Linear,
        )
        .unwrap(),
    );
    let moved = graph
        .evaluate_with_transforms([
            (
                child,
                AffineTransform::from_translation([0., 2., 0.]).unwrap(),
            ),
            (root, track.sample_transform(seconds(2.)).unwrap()),
        ])
        .unwrap();
    close(
        moved.node(child).unwrap().world.transform_point([0.; 3]),
        [4., 2., 0.],
    );
    assert_eq!(moved.node(root).unwrap().subtree_bounds, moved.bounds());
    close(moved.bounds().unwrap().min(), [3.5, 1.5, -0.5]);
    let ray = Ray::new([4.1, 2.2, 5.], [0., 0., -1.]).unwrap();
    let scene = moved.scene(Camera::default());
    let hit = scene.raycast(ray).unwrap();
    assert_eq!(hit.node, Some(child));
    assert_eq!(hit.object_id, Some("surface".into()));
    close(hit.normal, [0., 0., 1.]);
    assert!(authored.scene(Camera::default()).raycast(ray).is_none());
    let rewind = graph
        .evaluate_with_transforms([(root, track.sample_transform(Duration::ZERO).unwrap())])
        .unwrap();
    assert_eq!(rewind.bounds(), authored.bounds());
    assert_eq!(graph.revision(), authored.revision());
    assert_eq!(moved.revision(), authored.revision());
    assert_eq!(
        graph.node(child).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
    assert!(scene.raycast(ray).is_some());
    assert!(
        graph
            .evaluate()
            .unwrap()
            .scene(Camera::default())
            .raycast(ray)
            .is_none()
    );
}

#[test]
fn transform_overrides_reject_duplicates_foreign_and_expired_handles() {
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let pose = (node, AffineTransform::IDENTITY);
    assert!(
        matches!(graph.evaluate_with_transforms([pose, pose]), Err(SceneError::DuplicateTransform(handle)) if handle == node)
    );
    let other = SceneGraph::new();
    assert!(
        matches!(other.evaluate_with_transforms([pose]), Err(SceneError::InvalidHandle(handle)) if handle == node)
    );
    graph.remove_subtree(node).unwrap();
    let replacement = graph.insert(None, Node::new()).unwrap();
    assert!(
        matches!(graph.evaluate_with_transforms([pose]), Err(SceneError::InvalidHandle(handle)) if handle == node)
    );
    assert_eq!(
        graph.node(replacement).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
}

#[test]
fn invalid_world_composition_does_not_replace_authored_pose() {
    let mut graph = SceneGraph::new();
    let large = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [1e20; 3]).unwrap();
    let parent = graph.insert(None, Node::new()).unwrap();
    let child = graph
        .insert(Some(parent), Node::new().transform(large))
        .unwrap();
    let before = graph.evaluate().unwrap();
    assert!(
        matches!(graph.evaluate_with_transforms([(parent, large)]), Err(SceneError::InvalidTransform { node, .. }) if node == child)
    );
    assert_eq!(graph.revision(), before.revision());
    assert_eq!(
        graph.evaluate().unwrap().node(child).unwrap().world,
        before.node(child).unwrap().world
    );
}
