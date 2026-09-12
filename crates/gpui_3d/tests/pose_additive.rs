use gpui_3d::{
    AffineTransform, AnimationError, Interpolation, Keyframe, Node, Pose, PoseError, PoseMask,
    SceneGraph, TransformConstraint, TransformPose, TransformTrack, VectorTrack,
};
use std::{f32::consts::FRAC_1_SQRT_2, time::Duration};

fn translated(x: f32) -> TransformPose {
    TransformPose {
        translation: [x, 0., 0.],
        ..Default::default()
    }
}

fn close(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 2e-5, "{actual:?} != {expected:?}");
    }
}

#[test]
fn relative_trs_uses_parent_translation_and_multiplicative_signed_scale() {
    let base = TransformPose {
        translation: [10., -2., 3.],
        rotation: [0., 0., FRAC_1_SQRT_2, FRAC_1_SQRT_2],
        scale: [-2., 4., 1.],
    };
    let reference = TransformPose {
        translation: [2., 4., 6.],
        scale: [-2., 2., 4.],
        ..Default::default()
    };
    let sample = TransformPose {
        translation: [6., 2., 10.],
        scale: [-6., 1., 8.],
        ..reference
    };
    let result = base.additive(sample, reference, 0.5).unwrap();
    close(result.translation, [12., -3., 5.]);
    close(result.scale, [-4., 3., 1.5]);
    close(
        result.affine().unwrap().transform_point([1., 0., 0.]),
        [12., -7., 5.],
    );
    assert_eq!(base.additive(sample, reference, 0.).unwrap(), base);
    assert_eq!(base.additive(reference, reference, 0.7).unwrap(), base);
    assert_eq!(reference.additive(sample, reference, 1.).unwrap(), sample);
}

#[test]
fn relative_rotation_is_right_applied_and_layers_are_ordered() {
    let x = TransformPose {
        rotation: [FRAC_1_SQRT_2, 0., 0., FRAC_1_SQRT_2],
        ..Default::default()
    };
    let y = TransformPose {
        rotation: [0., FRAC_1_SQRT_2, 0., FRAC_1_SQRT_2],
        ..Default::default()
    };
    let z = TransformPose {
        rotation: [0., 0., FRAC_1_SQRT_2, FRAC_1_SQRT_2],
        ..Default::default()
    };
    let expected = x
        .affine()
        .unwrap()
        .compose(
            y.affine()
                .unwrap()
                .inverse()
                .compose(z.affine().unwrap())
                .unwrap(),
        )
        .unwrap();
    for sample in [
        z,
        TransformPose {
            rotation: z.rotation.map(|v| -v * 8.),
            ..z
        },
    ] {
        let actual = x.additive(sample, y, 1.).unwrap().affine().unwrap();
        for axis in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
            close(actual.transform_point(axis), expected.transform_point(axis));
        }
    }
    let identity = TransformPose::default();
    let half = identity
        .additive(z, identity, 0.5)
        .unwrap()
        .affine()
        .unwrap();
    close(
        half.transform_point([1., 0., 0.]),
        [FRAC_1_SQRT_2, FRAC_1_SQRT_2, 0.],
    );
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let base = Pose::new([(node, identity)]).unwrap();
    let a = Pose::new([(node, x)]).unwrap();
    let b = Pose::new([(node, y)]).unwrap();
    let ab = base
        .additive(&a, &base, 1., None)
        .unwrap()
        .additive(&b, &base, 1., None)
        .unwrap();
    let ba = base
        .additive(&b, &base, 1., None)
        .unwrap()
        .additive(&a, &base, 1., None)
        .unwrap();
    close(
        ab.get(node)
            .unwrap()
            .affine()
            .unwrap()
            .transform_point([0., 0., 1.]),
        [1., 0., 0.],
    );
    close(
        ba.get(node)
            .unwrap()
            .affine()
            .unwrap()
            .transform_point([0., 0., 1.]),
        [0., -1., 0.],
    );
}

#[test]
fn finite_extreme_inputs_preserve_small_translation_and_scale_results() {
    let base = translated(1e30);
    let reference = TransformPose {
        scale: [2.; 3],
        ..base
    };
    let sample = TransformPose {
        translation: [1e-20, 0., 0.],
        ..reference
    };
    assert_eq!(
        base.additive(sample, reference, 1.).unwrap().translation[0],
        1e-20
    );
    let base = translated(1e-20);
    let sample = TransformPose {
        scale: [3.; 3],
        ..reference
    };
    assert_eq!(
        base.additive(sample, reference, 0.5).unwrap().translation[0],
        1e-20
    );
    let scaled = |x| TransformPose {
        scale: [x, 1., 1.],
        ..Default::default()
    };
    let result = scaled(1e20)
        .additive(scaled(1e-27), scaled(1e30), 1.)
        .unwrap();
    assert!((result.scale[0] / 1e-37 - 1.).abs() < 1e-6);

    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let base = Pose::new([(node, TransformPose::default())]).unwrap();
    let sample = Pose::new([(node, translated(1e38))]).unwrap();
    let mask = PoseMask::new(1e-25, []).unwrap();
    let result = base.additive(&sample, &base, 1e-25, Some(&mask)).unwrap();
    assert!((result.get(node).unwrap().translation[0] / 1e-12 - 1.).abs() < 1e-6);
}

#[test]
fn invalid_inputs_scale_crossings_and_overflow_are_errors() {
    let identity = TransformPose::default();
    for weight in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert_eq!(
            identity.additive(identity, identity, weight),
            Err(AnimationError::InvalidBlendWeight)
        );
    }
    for invalid in [
        TransformPose {
            scale: [0., 1., 1.],
            ..identity
        },
        TransformPose {
            rotation: [0.; 4],
            ..identity
        },
        translated(f32::INFINITY),
    ] {
        for (base, sample, reference) in [
            (invalid, identity, identity),
            (identity, invalid, identity),
            (identity, identity, invalid),
        ] {
            assert!(matches!(
                base.additive(sample, reference, 0.),
                Err(AnimationError::InvalidTransform(_))
            ));
        }
    }
    let reflected = TransformPose {
        scale: [-1., 1., 1.],
        ..identity
    };
    assert!(matches!(
        identity.additive(reflected, identity, 0.5),
        Err(AnimationError::InvalidTransform(_))
    ));
    close(
        identity.additive(reflected, identity, 0.75).unwrap().scale,
        [-0.5, 1., 1.],
    );
    assert!(matches!(
        translated(f32::MAX).additive(translated(f32::MAX), identity, 1.),
        Err(AnimationError::InvalidTransform(_))
    ));
    let large = TransformPose {
        scale: [1e30, 1., 1.],
        ..identity
    };
    assert!(matches!(
        large.additive(large, identity, 1.),
        Err(AnimationError::InvalidTransform(_))
    ));
}

#[test]
fn sparse_reference_validation_and_failed_layers_preserve_all_inputs() {
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new()).unwrap();
    let b = graph.insert(None, Node::new()).unwrap();
    let c = graph.insert(None, Node::new()).unwrap();
    let identity = TransformPose::default();
    let base = Pose::new([(b, translated(3.)), (a, identity)]).unwrap();
    let sample = Pose::new([
        (a, translated(2.)),
        (
            b,
            TransformPose {
                scale: [-1., 1., 1.],
                ..identity
            },
        ),
    ])
    .unwrap();
    let reference = Pose::new([(c, identity), (a, identity), (b, identity)]).unwrap();
    let before: Vec<_> = [&base, &sample, &reference]
        .map(|pose| pose.poses().collect::<Vec<_>>())
        .into();
    assert!(
        matches!(base.additive(&sample, &reference, 0.5, None), Err(PoseError::InvalidPose { node, .. }) if node == b)
    );
    let after: Vec<_> = [&base, &sample, &reference]
        .map(|pose| pose.poses().collect::<Vec<_>>())
        .into();
    assert_eq!(before, after);
    let mask = PoseMask::new(1., [(b, 0.)]).unwrap();
    let mixed = base
        .additive(&sample, &reference, 0.5, Some(&mask))
        .unwrap();
    assert_eq!(
        mixed.poses().collect::<Vec<_>>(),
        vec![(b, translated(3.)), (a, translated(1.))]
    );
    let missing = Pose::new([(a, identity)]).unwrap();
    for weight in [0., 1.] {
        assert!(
            matches!(base.additive(&sample, &missing, weight, Some(&mask)), Err(PoseError::MissingReference(node)) if node == b)
        );
        assert!(
            matches!(missing.additive(&sample, &reference, weight, None), Err(PoseError::MissingNode(node)) if node == b)
        );
    }
    let foreign_mask = PoseMask::new(0., [(c, 0.)]).unwrap();
    assert!(
        matches!(base.additive(&missing, &reference, 0., Some(&foreign_mask)), Err(PoseError::MissingNode(node)) if node == c)
    );
}

#[test]
fn absolute_samples_and_local_masks_feed_final_hierarchy_and_constraints() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let child = graph.insert(Some(root), Node::new()).unwrap();
    let attached = graph.insert(None, Node::new()).unwrap();
    let base = Pose::new([(child, translated(1.)), (root, translated(10.))]).unwrap();
    let reference = Pose::new([(root, translated(2.))]).unwrap();
    let motion = TransformTrack::new(translated(2.)).unwrap().translation(
        VectorTrack::new(
            vec![
                Keyframe::new(Duration::ZERO, [2., 0., 0.]),
                Keyframe::new(Duration::from_secs(2), [6., 0., 0.]),
            ],
            Interpolation::Linear,
        )
        .unwrap(),
    );
    let mask = PoseMask::new(0., [(root, 0.5)]).unwrap();
    for time in [2., 1., 0., 1.5, 1.] {
        let sampled =
            Pose::new([(root, motion.sample(Duration::from_secs_f64(time)).unwrap())]).unwrap();
        let mixed = base
            .additive(&sampled, &reference, 0.5, Some(&mask))
            .unwrap();
        let evaluated = graph
            .evaluate_with_constraints(
                mixed.transforms(),
                [(
                    attached,
                    TransformConstraint::Follow {
                        target: child,
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
            [11. + time as f32 * 0.5, 0., 0.],
        );
        assert_eq!(mixed.get(child).unwrap(), translated(1.));
        assert_eq!(
            evaluated.node(child).unwrap().world,
            evaluated.node(attached).unwrap().world
        );
    }
}
