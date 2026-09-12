use gpui_3d::{
    AffineTransform, JointRotationLimitError as Error, JointRotationLimits, Node, SceneGraph,
    TransformPose,
};
use std::f32::consts::PI;

fn axis_angle(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let length = axis.iter().map(|v| v * v).sum::<f32>().sqrt();
    let sine = (angle * 0.5).sin() / length;
    [
        axis[0] * sine,
        axis[1] * sine,
        axis[2] * sine,
        (angle * 0.5).cos(),
    ]
}

fn compose(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = a;
    let [i, j, k, s] = b;
    [
        w * i + x * s + y * k - z * j,
        w * j - x * k + y * s + z * i,
        w * k + x * j - y * i + z * s,
        w * s - x * i - y * j - z * k,
    ]
}

fn near_rotation(a: [f32; 4], b: [f32; 4]) {
    let matrix = |q| {
        AffineTransform::from_trs([0.; 3], q, [1.; 3])
            .unwrap()
            .matrix()
    };
    for (a, b) in matrix(a).iter().flatten().zip(matrix(b).iter().flatten()) {
        assert!((a - b).abs() < 2e-6, "{a} != {b}");
    }
}

#[test]
fn limits_clamp_independent_components_in_the_reference_frame() {
    let reference = axis_angle([0., 0., 1.], 0.8);
    let settings = JointRotationLimits {
        reference_rotation: reference,
        twist_axis: [1., 2., 3.],
        max_swing: 0.5,
        twist_range: [-0.2, 0.3],
    };
    for (swing, twist) in [0_f32, 0.2, 1.2].into_iter().flat_map(|swing| {
        [-1.1_f32, 0.1, 0.9]
            .into_iter()
            .map(move |twist| (swing, twist))
    }) {
        let input = compose(
            reference,
            compose(
                axis_angle([2., -1., 0.], swing),
                axis_angle(settings.twist_axis, twist),
            ),
        );
        let result = settings.solve(input).unwrap();
        near_rotation(
            result.rotation,
            compose(
                reference,
                compose(
                    axis_angle([2., -1., 0.], swing.min(0.5)),
                    axis_angle(settings.twist_axis, twist.clamp(-0.2, 0.3)),
                ),
            ),
        );
        assert!((result.swing.requested_angle - f64::from(swing)).abs() < 1e-6);
        assert!((result.twist.requested_angle - f64::from(twist)).abs() < 1e-6);
        assert_eq!(result.swing.limited, swing > 0.5);
        assert_eq!(result.twist.limited, !(-0.2..=0.3).contains(&twist));
        assert!(!result.twist_degenerate);
        near_rotation(
            settings.solve(result.rotation).unwrap().rotation,
            result.rotation,
        );
        near_rotation(
            settings.solve(input.map(|v| -v)).unwrap().rotation,
            result.rotation,
        );
    }
}

#[test]
fn unrestricted_and_locked_limits_preserve_orientation_and_pose_components() {
    let settings = JointRotationLimits::default();
    for swing in [0., 0.3, 1.8, 3.1] {
        for twist in [-3.1, -0.7, 0., 2.8] {
            let input = compose(
                axis_angle([0., 1., 0.], swing),
                axis_angle([1., 0., 0.], twist),
            );
            let result = settings.solve(input).unwrap();
            near_rotation(result.rotation, input);
            assert!(!result.swing.limited && !result.twist.limited);
        }
    }
    let reference = axis_angle([1., 1., 0.], 0.7);
    let locked = JointRotationLimits {
        reference_rotation: reference,
        max_swing: 0.,
        twist_range: [0., 0.],
        ..settings
    };
    let mut pose = TransformPose {
        translation: [1., 2., 3.],
        rotation: axis_angle([0., 0., 1.], 1.3),
        scale: [-2., 3., 4.],
    };
    pose.rotation = locked.solve(pose.rotation).unwrap().rotation;
    near_rotation(pose.rotation, reference);
    let local = AffineTransform::from_trs(pose.translation, pose.rotation, pose.scale).unwrap();
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let evaluated = graph.evaluate_with_transforms([(node, local)]).unwrap();
    assert_eq!(
        evaluated.node(node).unwrap().world.transform_point([0.; 3]),
        pose.translation
    );
    let expected = AffineTransform::from_trs([1., 2., 3.], reference, [-2., 3., 4.]).unwrap();
    for (a, b) in local
        .matrix()
        .iter()
        .flatten()
        .zip(expected.matrix().iter().flatten())
    {
        assert!((a - b).abs() < 1e-5);
    }
}

#[test]
fn half_turns_have_deterministic_twist_and_degeneracy_diagnostics() {
    let settings = JointRotationLimits {
        max_swing: 0.6,
        twist_range: [-0.5, 0.25],
        ..Default::default()
    };
    for input in [[1., 0., 0., 0.], [-1., 0., 0., 0.]] {
        let result = settings.solve(input).unwrap();
        assert_eq!(result.twist.requested_angle, std::f64::consts::PI);
        assert!(!result.twist_degenerate);
        near_rotation(result.rotation, axis_angle([1., 0., 0.], 0.25));
    }
    for input in [[0., 1., 0., 0.], [0., -1., 0., 0.]] {
        let result = settings.solve(input).unwrap();
        assert!(result.twist_degenerate);
        assert_eq!(result.twist.requested_angle, 0.);
        near_rotation(result.rotation, axis_angle([0., 1., 0.], 0.6));
    }
    for angle in [-PI, PI] {
        let result = JointRotationLimits {
            twist_range: [angle, angle],
            ..settings
        }
        .solve([0., 0., 0., 1.])
        .unwrap();
        near_rotation(result.rotation, [1., 0., 0., 0.]);
    }
}

#[test]
fn finite_extreme_inputs_normalize_and_invalid_settings_return_errors() {
    let settings = JointRotationLimits::default();
    for size in [f32::from_bits(1), f32::MAX] {
        near_rotation(
            settings.solve([size, 0., 0., size]).unwrap().rotation,
            axis_angle([1., 0., 0.], PI * 0.5),
        );
        assert!(
            JointRotationLimits {
                twist_axis: [size, 0., 0.],
                ..settings
            }
            .solve([0., 0., 0., size])
            .is_ok()
        );
    }
    for rotation in [[0.; 4], [f32::NAN; 4], [f32::INFINITY; 4]] {
        assert_eq!(settings.solve(rotation), Err(Error::InvalidRotation));
        assert_eq!(
            JointRotationLimits {
                reference_rotation: rotation,
                ..settings
            }
            .solve([0., 0., 0., 1.]),
            Err(Error::InvalidReferenceRotation)
        );
    }
    for axis in [[0.; 3], [f32::NAN; 3], [f32::INFINITY; 3]] {
        assert_eq!(
            JointRotationLimits {
                twist_axis: axis,
                ..settings
            }
            .solve([0., 0., 0., 1.]),
            Err(Error::InvalidTwistAxis)
        );
    }
    for max_swing in [-0.1, PI + 0.1, f32::NAN, f32::INFINITY] {
        assert_eq!(
            JointRotationLimits {
                max_swing,
                ..settings
            }
            .solve([0., 0., 0., 1.]),
            Err(Error::InvalidSwingLimit)
        );
    }
    for twist_range in [
        [1., -1.],
        [-4., 0.],
        [0., 4.],
        [f32::NAN, 0.],
        [0., f32::INFINITY],
    ] {
        assert_eq!(
            JointRotationLimits {
                twist_range,
                ..settings
            }
            .solve([0., 0., 0., 1.]),
            Err(Error::InvalidTwistRange)
        );
    }
}
