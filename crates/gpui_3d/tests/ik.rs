use gpui_3d::{
    AffineTransform, IkOrientationTarget, IkReach, Node, SceneGraph, TwoBoneIkError,
    TwoBoneIkSettings,
};

fn translation(point: [f32; 3]) -> AffineTransform {
    AffineTransform::from_translation(point).unwrap()
}
fn chain(a: f32, b: f32) -> [AffineTransform; 3] {
    [
        translation([0.; 3]),
        translation([a, 0., 0.]),
        translation([a + b, 0., 0.]),
    ]
}
fn position(transform: AffineTransform) -> [f32; 3] {
    let m = transform.matrix();
    [m[3][0], m[3][1], m[3][2]]
}
fn close(a: [f32; 3], b: [f32; 3]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < 2e-5, "{a} != {b}");
    }
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f64 {
    a.into_iter()
        .zip(b)
        .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum::<f64>()
        .sqrt()
}
fn preserves_shape(before: [AffineTransform; 3], after: [AffineTransform; 3]) {
    assert_eq!(before[0].matrix()[3], after[0].matrix()[3]);
    for bone in 0..2 {
        let a = distance(position(before[bone]), position(before[bone + 1]));
        let b = distance(position(after[bone]), position(after[bone + 1]));
        assert!((a - b).abs() < 2e-5, "bone {bone}: {a} != {b}");
    }
    for (before, after) in before.into_iter().zip(after) {
        let determinant = |m: [[f32; 4]; 4]| {
            m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                - m[1][0] * (m[0][1] * m[2][2] - m[0][2] * m[2][1])
                + m[2][0] * (m[0][1] * m[1][2] - m[0][2] * m[1][1])
        };
        assert_eq!(
            determinant(before.matrix()).signum(),
            determinant(after.matrix()).signum()
        );
        for a in 0..3 {
            for b in 0..3 {
                let metric = |m: [[f32; 4]; 4]| (0..3).map(|r| m[a][r] * m[b][r]).sum::<f32>();
                assert!((metric(before.matrix()) - metric(after.matrix())).abs() < 2e-5);
            }
        }
    }
}

fn orientation_frame(transform: AffineTransform, axes: IkOrientationTarget) -> [[f32; 3]; 2] {
    let dot = |a: [f32; 3], b: [f32; 3]| a.into_iter().zip(b).map(|(a, b)| a * b).sum::<f32>();
    let unit = |v: [f32; 3]| v.map(|x| x / dot(v, v).sqrt());
    let matrix = transform.matrix();
    let direction = |v: [f32; 3]| {
        unit(std::array::from_fn(|r| {
            (0..3).map(|c| matrix[c][r] * v[c]).sum()
        }))
    };
    let forward = direction(axes.local_forward);
    let up = direction(axes.local_up);
    [
        forward,
        unit(std::array::from_fn(|i| {
            up[i] - dot(forward, up) * forward[i]
        })),
    ]
}

#[test]
fn terminal_orientation_preserves_position_solution_and_affine_shape() {
    let mut source = chain(1., 1.);
    source[2] = AffineTransform::from_matrix([
        [-2., 0., 0., 0.],
        [0.3, 1., 0., 0.],
        [0.2, 0.4, 1.5, 0.],
        [2., 0., 0., 1.],
    ])
    .unwrap();
    for target in [[1., 1., 0.], [5., 0., 0.]] {
        let settings = TwoBoneIkSettings { weight: 0.7 };
        let positional = settings.solve(source, target, [0., 0., 2.]).unwrap();
        assert!(positional.orientation.is_none());
        let orientation = IkOrientationTarget {
            rotation: [0.2, 0.4, -0.1, 0.8],
            local_forward: [2., 0., 0.],
            local_up: [0.5, 3., 0.],
            ..Default::default()
        };
        let result = settings
            .solve_with_orientation(source, target, [0., 0., 2.], orientation)
            .unwrap();
        assert_eq!(&result.transforms[..2], &positional.transforms[..2]);
        assert_eq!(
            result.transforms[2].matrix()[3],
            positional.transforms[2].matrix()[3]
        );
        assert_eq!(result.target_error, positional.target_error);
        assert_eq!(result.reach, positional.reach);
        assert_eq!(result.reachable_target, positional.reachable_target);
        preserves_shape(source, result.transforms);
        let desired = AffineTransform::from_trs([0.; 3], orientation.rotation, [1.; 3]).unwrap();
        for (actual, expected) in orientation_frame(result.transforms[2], orientation)
            .into_iter()
            .zip(orientation_frame(desired, orientation))
        {
            close(actual, expected);
        }
        assert!(result.orientation.unwrap().target_error < 1e-6);
        let repeated = settings
            .solve_with_orientation(source, target, [0., 0., 2.], orientation)
            .unwrap();
        assert_eq!(repeated, result);
    }
}

#[test]
fn terminal_orientation_weights_blend_shortest_rotations_independently() {
    let source = chain(1., 1.);
    for weight in [0., 0.25, 0.5, 1.] {
        let orientation = IkOrientationTarget {
            rotation: [0., 0., 1., 0.],
            weight,
            ..Default::default()
        };
        let settings = TwoBoneIkSettings { weight: 0. };
        let result = settings
            .solve_with_orientation(source, [1., 1., 0.], [0., 0., 2.], orientation)
            .unwrap();
        assert_eq!(&result.transforms[..2], &source[..2]);
        assert_eq!(result.transforms[2].matrix()[3], source[2].matrix()[3]);
        let angle = std::f32::consts::PI * weight;
        let desired = AffineTransform::from_trs(
            [2., 0., 0.],
            [0., 0., (angle / 2.).sin(), (angle / 2.).cos()],
            [1.; 3],
        )
        .unwrap();
        for (actual, expected) in result.transforms[2]
            .matrix()
            .iter()
            .flatten()
            .zip(desired.matrix().iter().flatten())
        {
            assert!((actual - expected).abs() < 2e-6);
        }
        let status = result.orientation.unwrap();
        assert!((status.requested_angle - std::f64::consts::PI).abs() < 1e-6);
        assert!((status.applied_angle - f64::from(angle)).abs() < 1e-6);
        assert!(
            (status.target_error - std::f64::consts::PI * (1. - f64::from(weight))).abs() < 1e-6
        );
        let negated = settings
            .solve_with_orientation(
                source,
                [1., 1., 0.],
                [0., 0., 2.],
                IkOrientationTarget {
                    rotation: orientation.rotation.map(|v| -v),
                    ..orientation
                },
            )
            .unwrap();
        assert_eq!(result, negated);
        if weight == 0. {
            assert_eq!(result.transforms, source);
        }
    }
    let settings = TwoBoneIkSettings::default();
    let positional = settings.solve(source, [1., 1., 0.], [0., 0., 2.]).unwrap();
    let result = settings
        .solve_with_orientation(
            source,
            [1., 1., 0.],
            [0., 0., 2.],
            IkOrientationTarget {
                weight: 0.,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(positional.transforms, result.transforms);
}

#[test]
fn terminal_orientation_validates_quaternions_weights_and_axis_frames() {
    let solve = |orientation| {
        TwoBoneIkSettings::default().solve_with_orientation(
            chain(1., 1.),
            [1., 1., 0.],
            [0., 0., 2.],
            orientation,
        )
    };
    for rotation in [[0.; 4], [f32::NAN; 4], [f32::INFINITY; 4]] {
        assert_eq!(
            solve(IkOrientationTarget {
                rotation,
                weight: 0.,
                ..Default::default()
            }),
            Err(TwoBoneIkError::InvalidOrientation)
        );
    }
    for weight in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            solve(IkOrientationTarget {
                weight,
                ..Default::default()
            }),
            Err(TwoBoneIkError::InvalidOrientationWeight)
        );
    }
    for local_up in [[0.; 3], [0., 0., 1.], [0., 1e-7, 1.], [f32::NAN; 3]] {
        assert_eq!(
            solve(IkOrientationTarget {
                local_up,
                weight: 0.,
                ..Default::default()
            }),
            Err(TwoBoneIkError::InvalidOrientationAxes)
        );
    }
    for value in [f32::from_bits(1), f32::MAX] {
        let result = solve(IkOrientationTarget {
            rotation: [0., 0., 0., value],
            ..Default::default()
        })
        .unwrap();
        assert!(result.orientation.unwrap().target_error < 1e-6);
    }
    let mut source = chain(1., 1.);
    source[2] = AffineTransform::from_trs([2., 0., 0.], [0., 0., 0., 1.], [1., 1e-8, 1.]).unwrap();
    assert_eq!(
        TwoBoneIkSettings { weight: 0. }.solve_with_orientation(
            source,
            [1., 1., 0.],
            [0., 0., 2.],
            IkOrientationTarget {
                local_forward: [1., 0., 0.],
                local_up: [1., 1., 0.],
                ..Default::default()
            },
        ),
        Err(TwoBoneIkError::InvalidOrientationAxes)
    );
}

#[test]
fn reachable_ik_uses_the_pole_and_retains_lengths() {
    let source = chain(1., 1.);
    for sign in [-1., 1.] {
        let result = TwoBoneIkSettings::default()
            .solve(source, [1., 1., 0.], [0., 0., sign * 2.])
            .unwrap();
        assert_eq!(result.reach, IkReach::Reachable);
        close(
            position(result.transforms[1]),
            [0.5, 0.5, sign * std::f32::consts::FRAC_1_SQRT_2],
        );
        close(position(result.transforms[2]), [1., 1., 0.]);
        close(result.reachable_target, [1., 1., 0.]);
        assert!(result.target_error < 2e-5);
        preserves_shape(source, result.transforms);
    }
}

#[test]
fn ik_clamps_both_reach_limits_and_supports_complete_folding() {
    let far = TwoBoneIkSettings::default()
        .solve(chain(1., 1.), [5., 0., 0.], [3., 0., 0.])
        .unwrap();
    assert_eq!(far.reach, IkReach::TooFar);
    close(far.reachable_target, [2., 0., 0.]);
    assert!((far.target_error - 3.).abs() < 1e-6);
    for (a, b, joint) in [(2., 1., [2., 0., 0.]), (1., 2., [-1., 0., 0.])] {
        let source = chain(a, b);
        for target in [[0.2, 0., 0.], [0.; 3]] {
            let result = TwoBoneIkSettings::default()
                .solve(source, target, [0., 2., 0.])
                .unwrap();
            assert_eq!(result.reach, IkReach::TooClose);
            close(result.reachable_target, [1., 0., 0.]);
            close(position(result.transforms[1]), joint);
            close(position(result.transforms[2]), [1., 0., 0.]);
            preserves_shape(source, result.transforms);
        }
    }
    let source = chain(1., 1.);
    for pole in [[0., 2., 0.], [0., 0., -2.]] {
        let result = TwoBoneIkSettings::default()
            .solve(source, [0.; 3], pole)
            .unwrap();
        assert_eq!(result.reach, IkReach::Reachable);
        close(
            position(result.transforms[1]),
            pole.map(|value| value * 0.5),
        );
        close(position(result.transforms[2]), [0.; 3]);
        preserves_shape(source, result.transforms);
    }
}

#[test]
fn ik_rotation_blending_preserves_affine_shapes_and_tip_local_pose() {
    let source = [
        AffineTransform::from_trs([0.; 3], [0.2, 0.3, 0.1, 0.9], [2., 0.5, -1.]).unwrap(),
        AffineTransform::from_matrix([
            [1., 0., 0., 0.],
            [0.3, 1., 0., 0.],
            [0., 0.2, 1., 0.],
            [1., 0., 0., 1.],
        ])
        .unwrap(),
        AffineTransform::from_trs([2., 0., 0.], [0.1, 0., 0.3, 0.9], [0.5, -1., 2.]).unwrap(),
    ];
    let tip_local = source[1].inverse().compose(source[2]).unwrap();
    let mut full = None;
    for weight in [1., 0.25, 0.5, 0., 0.75, 1.] {
        let result = TwoBoneIkSettings { weight }
            .solve(source, [0.5, 1., 0.25], [0., 0., 2.])
            .unwrap();
        preserves_shape(source, result.transforms);
        let actual_local = result.transforms[1]
            .inverse()
            .compose(result.transforms[2])
            .unwrap();
        for (actual, expected) in actual_local
            .matrix()
            .into_iter()
            .flatten()
            .zip(tip_local.matrix().into_iter().flatten())
        {
            assert!((actual - expected).abs() < 2e-5);
        }
        if weight == 0. {
            assert_eq!(result.transforms, source);
        }
        if weight == 1. {
            close(position(result.transforms[2]), [0.5, 1., 0.25]);
            if let Some(previous) = full {
                assert_eq!(result, previous);
            }
            full = Some(result);
        } else {
            assert!(result.target_error > 0.01);
        }
    }
}

#[test]
fn ik_reports_invalid_inputs_and_insufficient_position_precision() {
    let source = chain(1., 1.);
    for weight in [-1., 2., f32::NAN, f32::INFINITY] {
        assert_eq!(
            TwoBoneIkSettings { weight }.solve(source, [1., 1., 0.], [0., 0., 1.]),
            Err(TwoBoneIkError::InvalidWeight)
        );
    }
    assert_eq!(
        TwoBoneIkSettings::default().solve(source, [f32::NAN, 0., 0.], [0., 0., 1.]),
        Err(TwoBoneIkError::InvalidTarget)
    );
    for pole in [[0.; 3], [0., f32::INFINITY, 0.]] {
        assert_eq!(
            TwoBoneIkSettings::default().solve(source, [1., 1., 0.], pole),
            Err(TwoBoneIkError::InvalidPole)
        );
    }
    assert_eq!(
        TwoBoneIkSettings::default().solve(source, [1., 0., 0.], [2., 0., 0.]),
        Err(TwoBoneIkError::DegeneratePole)
    );
    for bone in 0..2 {
        let mut invalid = source;
        invalid[bone + 1] = invalid[bone];
        assert_eq!(
            TwoBoneIkSettings::default().solve(invalid, [1., 1., 0.], [0., 0., 1.]),
            Err(TwoBoneIkError::ZeroLengthBone { bone })
        );
    }
    let wide = [
        translation([100_000_000., 0., 0.]),
        translation([100_000_008., 0., 0.]),
        translation([100_000_016., 0., 0.]),
    ];
    assert_eq!(
        TwoBoneIkSettings::default().solve(wide, [100_000_008., 8., 0.], [100_000_000., 0., 10.]),
        Err(TwoBoneIkError::Unrepresentable)
    );
}

#[test]
fn solved_world_poses_apply_to_a_graph_without_changing_the_source_snapshot() {
    let mut graph = SceneGraph::new();
    let parent = graph
        .insert(None, Node::new().transform(translation([2., 0., 0.])))
        .unwrap();
    let root = graph.insert(Some(parent), Node::new()).unwrap();
    let middle = graph
        .insert(Some(root), Node::new().transform(translation([1., 0., 0.])))
        .unwrap();
    let tip = graph
        .insert(
            Some(middle),
            Node::new().transform(translation([1., 0., 0.])),
        )
        .unwrap();
    let attachment = graph
        .insert(Some(tip), Node::new().transform(translation([1., 0., 0.])))
        .unwrap();
    let snapshot = graph.evaluate().unwrap();
    let revision = graph.revision();
    let handles = [root, middle, tip];
    let source = handles.map(|node| snapshot.node(node).unwrap().world);
    let result = TwoBoneIkSettings::default()
        .solve_with_orientation(
            source,
            [3., 1., 0.],
            [2., 0., 2.],
            IkOrientationTarget {
                rotation: [
                    0.,
                    0.,
                    std::f32::consts::FRAC_1_SQRT_2,
                    std::f32::consts::FRAC_1_SQRT_2,
                ],
                ..Default::default()
            },
        )
        .unwrap();
    let [a, b, c] = result.transforms;
    let solved = graph
        .evaluate_with_transforms([
            (
                root,
                snapshot
                    .node(parent)
                    .unwrap()
                    .world
                    .inverse()
                    .compose(a)
                    .unwrap(),
            ),
            (middle, a.inverse().compose(b).unwrap()),
            (tip, b.inverse().compose(c).unwrap()),
        ])
        .unwrap();
    for (node, expected) in handles.into_iter().zip(result.transforms) {
        for (actual, expected) in solved
            .node(node)
            .unwrap()
            .world
            .matrix()
            .into_iter()
            .flatten()
            .zip(expected.matrix().into_iter().flatten())
        {
            assert!((actual - expected).abs() < 2e-5);
        }
    }
    close(position(solved.node(tip).unwrap().world), [3., 1., 0.]);
    close(
        position(solved.node(attachment).unwrap().world),
        [3., 2., 0.],
    );
    close(
        position(snapshot.node(attachment).unwrap().world),
        [5., 0., 0.],
    );
    close(position(snapshot.node(tip).unwrap().world), [4., 0., 0.]);
    assert_eq!(graph.revision(), revision);
    assert_eq!(
        graph.world_transform(tip).unwrap(),
        snapshot.node(tip).unwrap().world
    );
}

#[test]
fn ik_reaches_across_directions_length_ratios_and_nearly_folded_targets() {
    for (a, b) in [
        (1., 1.),
        (2., 1.),
        (1., 2.),
        (10., 0.1),
        (0.1, 10.),
        (0.001, 0.002),
    ] {
        let source = chain(a, b);
        for direction in [[1., 0., 0.], [-1., 0., 0.], [0., 1., 0.], [0.3, -0.4, 0.5]] {
            let length = distance(direction, [0.; 3]) as f32;
            let direction = direction.map(|value| value / length);
            for ratio in [0.01, 0.5, 1.1] {
                let target = direction.map(|value| value * (a + b) * ratio);
                let result = TwoBoneIkSettings::default()
                    .solve(source, target, [0.1, 0.2, 2.])
                    .unwrap();
                preserves_shape(source, result.transforms);
                close(position(result.transforms[2]), result.reachable_target);
                if result.reach == IkReach::Reachable {
                    close(position(result.transforms[2]), target);
                }
            }
        }
    }
    let source = chain(100_000_000., 100_000_000.);
    let target = [1e-8, 0., 0.];
    let result = TwoBoneIkSettings::default()
        .solve(source, target, [0., 1., 0.])
        .unwrap();
    assert_eq!(result.reach, IkReach::Reachable);
    assert_eq!(position(result.transforms[2]), target);
    assert_eq!(result.target_error, 0.);
    preserves_shape(source, result.transforms);
}

#[test]
fn bent_source_poses_and_rigid_reference_frames_produce_consistent_solutions() {
    let source = [
        translation([0.; 3]),
        translation([1., 0., 0.]),
        translation([1., 1., 0.]),
    ];
    for weight in [0.5, 1.] {
        let settings = TwoBoneIkSettings { weight };
        let base = settings.solve(source, [0., 0., 1.], [1., 0., 0.]).unwrap();
        for scale in [[1.; 3], [-1., 1., 1.]] {
            let reference =
                AffineTransform::from_trs([2., 3., -1.], [0.3, 0.2, 0.5, 0.8], scale).unwrap();
            let moved = source.map(|pose| reference.compose(pose).unwrap());
            let target = reference.transform_point([0., 0., 1.]);
            let result = settings
                .solve(moved, target, reference.transform_point([1., 0., 0.]))
                .unwrap();
            preserves_shape(moved, result.transforms);
            for (actual, expected) in result
                .transforms
                .into_iter()
                .zip(base.transforms.map(|pose| reference.compose(pose).unwrap()))
            {
                for (a, b) in actual
                    .matrix()
                    .into_iter()
                    .flatten()
                    .zip(expected.matrix().into_iter().flatten())
                {
                    assert!((a - b).abs() < 2e-5, "{a} != {b}");
                }
            }
            if weight == 1. {
                close(position(result.transforms[2]), target);
            }
        }
        let local_tip = base.transforms[1]
            .inverse()
            .compose(base.transforms[2])
            .unwrap();
        let expected = source[1].inverse().compose(source[2]).unwrap();
        for (a, b) in local_tip
            .matrix()
            .into_iter()
            .flatten()
            .zip(expected.matrix().into_iter().flatten())
        {
            assert!((a - b).abs() < 2e-5);
        }
    }
}
