use gpui_3d::{
    AffineTransform, IkChainError, IkChainJoint, IkChainSettings, IkChainStatus,
    JointRotationLimits, Node, SceneGraph, TransformPose,
};

fn chain(count: usize) -> Vec<IkChainJoint> {
    (0..count)
        .map(|joint| {
            TransformPose {
                translation: if joint == 0 { [0.; 3] } else { [1., 0., 0.] },
                ..Default::default()
            }
            .into()
        })
        .collect()
}

fn point(transform: AffineTransform) -> [f32; 3] {
    transform.transform_point([0.; 3])
}

fn close(a: [f32; 3], b: [f32; 3], tolerance: f32) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < tolerance, "{a} != {b}");
    }
}

#[test]
fn constrained_ccd_reaches_spatial_targets_and_escapes_straight_chain_stalls() {
    let source = chain(4);
    let unchanged = source.clone();
    for target in [[1.5, 0., 0.], [1., 1., 1.], [-1., 0.5, 0.], [0., 0., -2.]] {
        let settings = IkChainSettings::default();
        let result = settings
            .solve(AffineTransform::IDENTITY, &source, target)
            .unwrap();
        assert_eq!(
            result.status,
            IkChainStatus::Converged,
            "{target:?}: {result:?}"
        );
        assert!(result.target_error <= f64::from(settings.tolerance));
        assert!(result.iterations <= settings.max_iterations);
        close(point(*result.transforms.last().unwrap()), target, 1.1e-4);
        assert_eq!(point(result.transforms[0]), [0.; 3]);
        for pair in result.transforms.windows(2) {
            let squared: f32 = point(pair[0])
                .into_iter()
                .zip(point(pair[1]))
                .map(|(a, b)| (a - b).powi(2))
                .sum();
            assert!((squared - 1.).abs() < 2e-5);
        }
        assert_eq!(
            result,
            settings
                .solve(AffineTransform::IDENTITY, &source, target)
                .unwrap()
        );
    }
    assert_eq!(source, unchanged);
}

#[test]
fn every_accepted_pose_respects_local_hinges_and_retains_translation_and_scale() {
    let mut source = chain(4);
    let limits = JointRotationLimits {
        twist_axis: [0., 0., 1.],
        max_swing: 0.,
        twist_range: [0., std::f32::consts::FRAC_PI_2],
        ..Default::default()
    };
    for joint in &mut source {
        joint.limits = Some(limits);
    }
    let result = IkChainSettings {
        max_iterations: 256,
        ..Default::default()
    }
    .solve(AffineTransform::IDENTITY, &source, [1.5, 1.5, 0.])
    .unwrap();
    assert_eq!(result.status, IkChainStatus::Converged, "{result:?}");
    for (actual, expected) in result.poses.iter().zip(&source) {
        assert_eq!(actual.translation, expected.pose.translation);
        assert_eq!(actual.scale, expected.pose.scale);
        let angles = limits.solve(actual.rotation).unwrap();
        assert!(angles.swing.requested_angle < 1e-6);
        assert!(angles.twist.requested_angle >= -1e-6);
        assert!(angles.twist.requested_angle <= std::f64::consts::FRAC_PI_2 + 1e-6);
    }
}

#[test]
fn zero_iteration_projects_all_limits_and_reports_the_returned_pose_error() {
    let mut source = chain(3);
    let locked = JointRotationLimits {
        max_swing: 0.,
        twist_range: [0., 0.],
        ..Default::default()
    };
    for joint in &mut source {
        joint.pose.rotation = [0., 0., 1., 1.];
        joint.limits = Some(locked);
    }
    let settings = IkChainSettings {
        max_iterations: 0,
        ..Default::default()
    };
    let result = settings
        .solve(AffineTransform::IDENTITY, &source, [1., 1., 0.])
        .unwrap();
    assert_eq!(result.status, IkChainStatus::IterationLimit);
    assert_eq!(result.iterations, 0);
    assert_eq!(result.limited_joints, [0, 1, 2]);
    close(point(result.transforms[2]), [2., 0., 0.], 1e-6);
    assert!((result.target_error - 2_f64.sqrt()).abs() < 1e-6);
    let reached = settings
        .solve(AffineTransform::IDENTITY, &source, [2., 0., 0.])
        .unwrap();
    assert_eq!(reached.status, IkChainStatus::Converged);
    let stalled = IkChainSettings::default()
        .solve(AffineTransform::IDENTITY, &source, [1., 1., 0.])
        .unwrap();
    assert_eq!(stalled.status, IkChainStatus::Stalled);
    assert_eq!(stalled.target_error, result.target_error);
}

#[test]
fn nonconvergence_does_not_claim_unreachable_or_return_a_worse_pose() {
    let source = chain(3);
    let target = [8., 2., 0.];
    let result = IkChainSettings::default()
        .solve(AffineTransform::IDENTITY, &source, target)
        .unwrap();
    assert_ne!(result.status, IkChainStatus::Converged);
    let initial_error = (6_f64.powi(2) + 2_f64.powi(2)).sqrt();
    assert!(result.target_error <= initial_error);
    let measured = point(*result.transforms.last().unwrap())
        .into_iter()
        .zip(target)
        .map(|(a, b)| (f64::from(a) - f64::from(b)).powi(2))
        .sum::<f64>()
        .sqrt();
    assert_eq!(result.target_error, measured);
    let budget = IkChainSettings {
        max_iterations: 1,
        tolerance: 0.,
        max_step_angle: 0.01,
    }
    .solve(AffineTransform::IDENTITY, &source, [0., 2., 0.])
    .unwrap();
    assert_eq!(budget.status, IkChainStatus::IterationLimit);
    assert_eq!(budget.iterations, 1);
    let stopped = IkChainSettings {
        max_step_angle: 0.,
        ..Default::default()
    }
    .solve(AffineTransform::IDENTITY, &source, [0., 2., 0.])
    .unwrap();
    assert_eq!(stopped.status, IkChainStatus::Stalled);
    assert_eq!(stopped.iterations, 0);
    assert_eq!(
        stopped.poses,
        source.iter().map(|joint| joint.pose).collect::<Vec<_>>()
    );
    let coincident: Vec<IkChainJoint> = vec![TransformPose::default().into(); 4];
    let fixed = IkChainSettings::default()
        .solve(AffineTransform::IDENTITY, &coincident, [1., 0., 0.])
        .unwrap();
    assert_eq!(fixed.status, IkChainStatus::Stalled);
    assert_eq!(fixed.target_error, 1.);
}

#[test]
fn returned_local_poses_reproduce_world_transforms_under_affine_parents() {
    let parent = AffineTransform::from_matrix([
        [-2., 0., 0., 0.],
        [0.4, 1., 0., 0.],
        [0., 0.2, 1.5, 0.],
        [4., 2., 1., 1.],
    ])
    .unwrap();
    let mut source = chain(4);
    source[1].pose.scale = [1., 0.8, -1.];
    let mut graph = SceneGraph::new();
    let external = graph.insert(None, Node::new().transform(parent)).unwrap();
    let mut previous = external;
    let handles: Vec<_> = source
        .iter()
        .map(|joint| {
            let handle = graph
                .insert(
                    Some(previous),
                    Node::new().transform(joint.pose.affine().unwrap()),
                )
                .unwrap();
            previous = handle;
            handle
        })
        .collect();
    let before = graph.evaluate().unwrap();
    let result = IkChainSettings {
        max_iterations: 256,
        ..Default::default()
    }
    .solve(parent, &source, [0., 3., 1.])
    .unwrap();
    let after = graph
        .evaluate_with_transforms(
            handles
                .iter()
                .zip(&result.poses)
                .map(|(&handle, pose)| (handle, pose.affine().unwrap())),
        )
        .unwrap();
    for ((&handle, world), (pose, initial)) in handles
        .iter()
        .zip(&result.transforms)
        .zip(result.poses.iter().zip(&source))
    {
        assert_eq!(after.node(handle).unwrap().world, *world);
        assert_eq!(pose.translation, initial.pose.translation);
        assert_eq!(pose.scale, initial.pose.scale);
        assert_eq!(
            graph.world_transform(handle).unwrap(),
            before.node(handle).unwrap().world
        );
    }
    assert!(result.target_error < 1e-3, "{result:?}");
}

#[test]
fn chain_validation_identifies_invalid_joints_and_settings() {
    let settings = IkChainSettings::default();
    let solve =
        |joints: &[IkChainJoint]| settings.solve(AffineTransform::IDENTITY, joints, [1., 1., 0.]);
    assert_eq!(solve(&[]), Err(IkChainError::TooFewJoints));
    assert_eq!(solve(&chain(1)), Err(IkChainError::TooFewJoints));
    let mut source = chain(3);
    source[1].pose.scale = [0.; 3];
    assert!(matches!(
        solve(&source),
        Err(IkChainError::InvalidPose { joint: 1, .. })
    ));
    source = chain(3);
    source[2].limits = Some(JointRotationLimits {
        max_swing: f32::NAN,
        ..Default::default()
    });
    assert!(matches!(
        solve(&source),
        Err(IkChainError::InvalidLimit { joint: 2, .. })
    ));
    source = chain(3);
    for target in [[f32::NAN; 3], [f32::INFINITY; 3]] {
        assert_eq!(
            settings.solve(AffineTransform::IDENTITY, &source, target),
            Err(IkChainError::InvalidTarget)
        );
    }
    for tolerance in [-0.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            IkChainSettings {
                tolerance,
                ..settings
            }
            .solve(AffineTransform::IDENTITY, &source, [0.; 3]),
            Err(IkChainError::InvalidTolerance)
        );
    }
    for max_step_angle in [-0.1, 4., f32::NAN, f32::INFINITY] {
        assert_eq!(
            IkChainSettings {
                max_step_angle,
                ..settings
            }
            .solve(AffineTransform::IDENTITY, &source, [0.; 3]),
            Err(IkChainError::InvalidStepAngle)
        );
    }
    source[1].pose.translation = [f32::MAX, 0., 0.];
    let parent = AffineTransform::from_translation([f32::MAX, 0., 0.]).unwrap();
    assert_eq!(
        settings.solve(parent, &source, [0.; 3]),
        Err(IkChainError::Unrepresentable { joint: 1 })
    );
}
