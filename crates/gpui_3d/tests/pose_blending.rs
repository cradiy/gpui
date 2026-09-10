use gpui_3d::{
    AffineTransform, AnimationError, Interpolation, Keyframe, Node, Pose, PoseError, PoseMask,
    SceneGraph, TransformConstraint, TransformPose, TransformTrack, VectorTrack,
};
use std::time::Duration;

fn translated(x: f32, y: f32, z: f32) -> TransformPose {
    TransformPose {
        translation: [x, y, z],
        ..Default::default()
    }
}

fn close(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 2e-5, "{actual:?} != {expected:?}");
    }
}

#[test]
fn local_blending_uses_shortest_arc_with_linear_translation_and_signed_scale() {
    let base = TransformPose {
        translation: [-2., 1., 4.],
        rotation: [0., 0., 0., 2.],
        scale: [-1., 1., 2.],
    };
    let target = TransformPose {
        translation: [6., -3., 0.],
        rotation: [0., 0., -3_f32.sqrt(), -1.],
        scale: [-3., 3., 4.],
    };
    assert_eq!(base.blend(target, 0.).unwrap(), base);
    assert_eq!(base.blend(target, 1.).unwrap(), target);
    for (weight, degrees) in [(0.25, 30_f32), (0.75, 90.), (0.5, 60.)] {
        let pose = base.blend(target, weight).unwrap();
        close(
            pose.translation,
            [-2. + 8. * weight, 1. - 4. * weight, 4. - 4. * weight],
        );
        close(
            pose.scale,
            [-1. - 2. * weight, 1. + 2. * weight, 2. + 2. * weight],
        );
        let rotation = AffineTransform::from_trs([0.; 3], pose.rotation, [1.; 3]).unwrap();
        close(
            rotation.transform_point([1., 0., 0.]),
            [degrees.to_radians().cos(), degrees.to_radians().sin(), 0.],
        );
    }
    let a = TransformPose {
        rotation: [0., f32::MIN_POSITIVE, 0., 0.],
        ..Default::default()
    };
    let b = TransformPose {
        rotation: [0., -f32::MAX, 0., 0.],
        ..Default::default()
    };
    close(
        a.blend(b, 0.5)
            .unwrap()
            .affine()
            .unwrap()
            .transform_point([1., 0., 0.]),
        [-1., 0., 0.],
    );
}

#[test]
fn invalid_inputs_and_singular_scale_crossings_return_errors() {
    let base = TransformPose::default();
    for weight in [-1., 1.01, f32::NAN, f32::INFINITY] {
        assert_eq!(
            base.blend(base, weight),
            Err(AnimationError::InvalidBlendWeight)
        );
    }
    for invalid in [
        TransformPose {
            rotation: [0.; 4],
            ..base
        },
        TransformPose {
            scale: [0., 1., 1.],
            ..base
        },
        translated(f32::INFINITY, 0., 0.),
    ] {
        assert!(matches!(
            base.blend(invalid, 0.),
            Err(AnimationError::InvalidTransform(_))
        ));
        assert!(matches!(
            invalid.blend(base, 1.),
            Err(AnimationError::InvalidTransform(_))
        ));
    }
    let reflected = TransformPose {
        scale: [-1., 1., 1.],
        ..base
    };
    assert!(matches!(
        base.blend(reflected, 0.5),
        Err(AnimationError::InvalidTransform(_))
    ));
    assert!(base.blend(reflected, 0.25).is_ok());
    let a = translated(-f32::MAX, 0., 0.);
    let b = translated(f32::MAX, 0., 0.);
    assert_eq!(a.blend(b, 0.5).unwrap().translation, [0.; 3]);
}

#[test]
fn sparse_targets_and_local_masks_preserve_base_order_and_parent_inheritance() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let a = graph.insert(Some(root), Node::new()).unwrap();
    let b = graph.insert(Some(root), Node::new()).unwrap();
    let base = Pose::new([
        (b, translated(-1., 0., 0.)),
        (root, TransformPose::default()),
        (a, translated(0., 1., 0.)),
    ])
    .unwrap();
    let target = Pose::new([(a, translated(0., 5., 0.)), (root, translated(4., 0., 0.))]).unwrap();
    let mask = PoseMask::new(0., [(root, 1.), (a, 0.5)]).unwrap();
    let mixed = base.blend(&target, 0.5, Some(&mask)).unwrap();
    assert_eq!(
        mixed.poses().map(|(node, _)| node).collect::<Vec<_>>(),
        vec![b, root, a]
    );
    assert_eq!(mixed.get(root).unwrap().translation, [2., 0., 0.]);
    assert_eq!(mixed.get(a).unwrap().translation, [0., 2., 0.]);
    assert_eq!(mixed.get(b), base.get(b));
    let evaluated = graph.evaluate_with_transforms(mixed.transforms()).unwrap();
    assert_eq!(
        evaluated.node(b).unwrap().world.transform_point([0.; 3]),
        [1., 0., 0.]
    );
    assert_eq!(
        evaluated.node(a).unwrap().world.transform_point([0.; 3]),
        [2., 2., 0.]
    );
    assert_eq!(base.get(root).unwrap(), TransformPose::default());
    assert_eq!(target.get(a).unwrap().translation, [0., 5., 0.]);
    let excluded = PoseMask::new(0., []).unwrap();
    assert_eq!(
        base.blend(&target, 1., Some(&excluded))
            .unwrap()
            .poses()
            .collect::<Vec<_>>(),
        base.poses().collect::<Vec<_>>()
    );
    let all = PoseMask::new(1., [(a, 0.)]).unwrap();
    let mixed = base.blend(&target, 1., Some(&all)).unwrap();
    assert_eq!(mixed.get(root), target.get(root));
    assert_eq!(mixed.get(a), base.get(a));
}

#[test]
fn masked_weights_keep_small_products_before_applying_large_pose_values() {
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let base = Pose::new([(node, TransformPose::default())]).unwrap();
    let target = Pose::new([(node, translated(1e38, 0., 0.))]).unwrap();
    let mask = PoseMask::new(1e-25, []).unwrap();
    let mixed = base.blend(&target, 1e-25, Some(&mask)).unwrap();
    let expected = (f64::from(1e38_f32) * f64::from(1e-25_f32).powi(2)) as f32;
    assert!(mixed.get(node).unwrap().translation[0] > 0.);
    assert_eq!(mixed.get(node).unwrap().translation[0], expected);
}

#[test]
fn invalid_pose_references_weights_and_failed_layers_preserve_inputs() {
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new()).unwrap();
    let b = graph.insert(None, Node::new()).unwrap();
    let c = graph.insert(None, Node::new()).unwrap();
    let base = Pose::new([(a, TransformPose::default()), (b, translated(1., 0., 0.))]).unwrap();
    let before = base.poses().collect::<Vec<_>>();
    assert_eq!(
        Pose::new([(a, TransformPose::default()); 2]).unwrap_err(),
        PoseError::DuplicateNode(a)
    );
    assert_eq!(
        PoseMask::new(0., [(a, 0.), (a, 1.)]).unwrap_err(),
        PoseError::DuplicateNode(a)
    );
    for weight in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            PoseMask::new(weight, []).unwrap_err(),
            PoseError::InvalidWeight { node: None }
        );
        assert_eq!(
            PoseMask::new(0., [(a, weight)]).unwrap_err(),
            PoseError::InvalidWeight { node: Some(a) }
        );
        assert_eq!(
            base.blend(&Pose::default(), weight, None).unwrap_err(),
            PoseError::InvalidWeight { node: None }
        );
    }
    let extra = Pose::new([(c, TransformPose::default())]).unwrap();
    assert_eq!(
        base.blend(&extra, 0., None).unwrap_err(),
        PoseError::MissingNode(c)
    );
    let mask = PoseMask::new(1., [(c, 0.)]).unwrap();
    assert_eq!(
        base.blend(&base, 0., Some(&mask)).unwrap_err(),
        PoseError::MissingNode(c)
    );
    let target = Pose::new([
        (a, translated(8., 0., 0.)),
        (
            b,
            TransformPose {
                scale: [-1., 1., 1.],
                ..Default::default()
            },
        ),
    ])
    .unwrap();
    assert!(
        matches!(base.blend(&target, 0.5, None), Err(PoseError::InvalidPose { node, .. }) if node == b)
    );
    assert_eq!(base.poses().collect::<Vec<_>>(), before);
    graph.remove_subtree(a).unwrap();
    assert!(graph.evaluate_with_transforms(base.transforms()).is_err());
}

#[test]
fn ordered_override_layers_and_random_time_sampling_feed_final_constraint_poses() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let joint = graph.insert(Some(root), Node::new()).unwrap();
    let tip = graph
        .insert(
            Some(joint),
            Node::new().transform(AffineTransform::from_translation([0., 1., 0.]).unwrap()),
        )
        .unwrap();
    let attached = graph.insert(None, Node::new()).unwrap();
    let base = Pose::new([
        (root, TransformPose::default()),
        (joint, translated(0., 1., 0.)),
    ])
    .unwrap();
    let motion = TransformTrack::new(TransformPose::default())
        .unwrap()
        .translation(
            VectorTrack::new(
                [
                    Keyframe::new(Duration::ZERO, [0.; 3]),
                    Keyframe::new(Duration::from_secs(2), [4., 0., 0.]),
                ],
                Interpolation::Linear,
            )
            .unwrap(),
        );
    let turn = Pose::new([(
        joint,
        TransformPose {
            rotation: [
                0.,
                0.,
                std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2,
            ],
            ..translated(0., 1., 0.)
        },
    )])
    .unwrap();
    for time in [2., 1., 0., 1.5, 1.] {
        let sampled =
            Pose::new([(root, motion.sample(Duration::from_secs_f64(time)).unwrap())]).unwrap();
        let mixed = base
            .blend(&sampled, 0.5, None)
            .unwrap()
            .blend(&turn, 0.5, None)
            .unwrap();
        let evaluated = graph
            .evaluate_with_constraints(
                mixed.transforms(),
                [(
                    attached,
                    TransformConstraint::Follow {
                        target: tip,
                        offset: AffineTransform::IDENTITY,
                    },
                )],
            )
            .unwrap();
        close(
            evaluated
                .node(attached)
                .unwrap()
                .world
                .transform_point([0.; 3]),
            [
                time as f32 - std::f32::consts::FRAC_1_SQRT_2,
                1. + std::f32::consts::FRAC_1_SQRT_2,
                0.,
            ],
        );
        assert_eq!(
            evaluated.node(tip).unwrap().world,
            evaluated.node(attached).unwrap().world
        );
    }
    let first = Pose::new([(root, translated(4., 0., 0.))]).unwrap();
    let second = Pose::new([(root, translated(8., 0., 0.))]).unwrap();
    assert_eq!(
        base.blend(&first, 0.5, None)
            .unwrap()
            .blend(&second, 0.5, None)
            .unwrap()
            .get(root)
            .unwrap()
            .translation[0],
        5.
    );
    assert_eq!(
        base.blend(&second, 0.5, None)
            .unwrap()
            .blend(&first, 0.5, None)
            .unwrap()
            .get(root)
            .unwrap()
            .translation[0],
        4.
    );
}
