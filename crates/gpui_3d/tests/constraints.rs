use gpui::rgb;
use gpui_3d::{
    AffineTransform, AimError, AimSettings, Camera, ConstraintStatus, Interpolation, Keyframe,
    Material, Mesh, Node, NodeHandle, Ray, ResolvedTexture, SceneError, SceneGraph, TextureState,
    TransformConstraint, TransformPose, TransformTrack, VectorTrack,
};
use std::time::Duration;

fn translate(position: [f32; 3]) -> AffineTransform {
    AffineTransform::from_translation(position).unwrap()
}

fn follow(target: NodeHandle, offset: AffineTransform) -> TransformConstraint {
    TransformConstraint::Follow { target, offset }
}

fn close(actual: AffineTransform, expected: AffineTransform) {
    for (actual, expected) in actual
        .matrix()
        .into_iter()
        .flatten()
        .zip(expected.matrix().into_iter().flatten())
    {
        assert!((actual - expected).abs() < 1e-5, "{actual} != {expected}");
    }
}

#[test]
fn follow_dependencies_preserve_affine_offsets_hierarchy_and_render_identity() {
    let mut graph = SceneGraph::new();
    let parent = graph
        .insert(None, Node::new().transform(translate([8., 0., 0.])))
        .unwrap();
    let mesh = Mesh::cube();
    let attached = graph
        .insert(
            Some(parent),
            Node::new()
                .id("attached")
                .transform(translate([5., 0., 0.]))
                .mesh(mesh.clone(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let child = graph
        .insert(
            Some(attached),
            Node::new()
                .transform(translate([0., 2., 0.]))
                .camera(Camera {
                    eye: [0.; 3],
                    target: [0., 0., -1.],
                    ..Default::default()
                }),
        )
        .unwrap();
    let anchor = graph.insert(None, Node::new()).unwrap();
    let driver = graph.insert(None, Node::new()).unwrap();
    let source = graph
        .insert(Some(driver), Node::new().visible(false))
        .unwrap();
    let driver_pose = translate([0., -0.25, 0.]);
    let offset = AffineTransform::from_matrix([
        [1., 0., 0., 0.],
        [0.3, 1., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0.5, 0., 1.],
    ])
    .unwrap();
    let pose =
        AffineTransform::from_trs([1., 0., 0.], [0.1, 0.3, 0.2, 0.9], [-1., 2., 0.5]).unwrap();
    let bindings = [
        (attached, follow(anchor, offset)),
        (anchor, follow(source, translate([0., 1., 0.]))),
    ];
    let revision = graph.revision();
    let original = graph.evaluate().unwrap();
    let result = graph
        .evaluate_with_constraints(
            [
                (driver, driver_pose),
                (source, pose),
                (attached, translate([100., 0., 0.])),
            ],
            bindings,
        )
        .unwrap();
    let reversed = graph
        .evaluate_with_constraints(
            [(driver, driver_pose), (source, pose)],
            bindings.into_iter().rev(),
        )
        .unwrap();
    let expected = driver_pose
        .compose(pose)
        .unwrap()
        .compose(translate([0., 1., 0.]))
        .unwrap()
        .compose(offset)
        .unwrap();
    close(result.node(attached).unwrap().world, expected);
    close(
        result.node(child).unwrap().world,
        expected.compose(translate([0., 2., 0.])).unwrap(),
    );
    let child_camera = result.node(child).unwrap().camera.unwrap();
    for (actual, expected) in child_camera
        .eye
        .into_iter()
        .zip(expected.transform_point([0., 2., 0.]))
    {
        assert!((actual - expected).abs() < 1e-5);
    }
    assert!(result.node(attached).unwrap().visible);
    assert!(!result.node(source).unwrap().visible);
    assert_eq!(result.node(attached).unwrap().parent, Some(parent));
    assert_eq!(
        result.node(attached).unwrap().bounds,
        Some(mesh.bounds().transformed(expected).unwrap())
    );
    for ((node, reversed), original) in result
        .nodes()
        .iter()
        .zip(reversed.nodes())
        .zip(original.nodes())
    {
        assert_eq!(node.handle, original.handle);
        assert_eq!(node.handle, reversed.handle);
        close(node.world, reversed.world);
    }
    let camera = Camera::default()
        .frame_bounds(result.bounds().unwrap(), 1., 1.5)
        .unwrap();
    let scene = result.scene(camera);
    let prepared = scene
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(prepared.objects()[0].node, Some(attached));
    assert_eq!(prepared.frame().objects[0].model, expected.matrix());
    assert_eq!(prepared.frame().objects[0].normal, expected.normal_matrix());
    let origin = expected.transform_point([0., 0., 3.]);
    let center = expected.transform_point([0.; 3]);
    let hit = scene
        .raycast(Ray::new(origin, std::array::from_fn(|i| center[i] - origin[i])).unwrap())
        .unwrap();
    assert_eq!(hit.node, Some(attached));
    assert_eq!(graph.revision(), revision);
    close(
        graph.world_transform(attached).unwrap(),
        original.node(attached).unwrap().world,
    );
    close(
        original.node(attached).unwrap().world,
        translate([13., 0., 0.]),
    );
    graph.set_visible(parent, false).unwrap();
    let hidden = graph
        .evaluate_with_constraints([(driver, driver_pose), (source, pose)], bindings)
        .unwrap();
    assert!(!hidden.node(attached).unwrap().visible);
    assert!(!hidden.node(child).unwrap().visible);
    close(hidden.node(attached).unwrap().world, expected);
}

#[test]
fn animated_follow_can_seek_and_release_without_changing_prior_snapshots() {
    let mut graph = SceneGraph::new();
    let parent = graph
        .insert(None, Node::new().transform(translate([10., 0., 0.])))
        .unwrap();
    let follower = graph
        .insert(Some(parent), Node::new().transform(translate([2., 0., 0.])))
        .unwrap();
    let target = graph.insert(None, Node::new()).unwrap();
    let track = TransformTrack::new(TransformPose::default())
        .unwrap()
        .translation(
            VectorTrack::new(
                [
                    Keyframe::new(Duration::ZERO, [0.; 3]),
                    Keyframe::new(Duration::from_secs(4), [4., 0., 0.]),
                ],
                Interpolation::Linear,
            )
            .unwrap(),
        );
    let mut snapshots = Vec::new();
    for time in [3, 1, 4, 0, 3] {
        let local = track.sample_transform(Duration::from_secs(time)).unwrap();
        let snapshot = graph
            .evaluate_with_constraints(
                [(target, local)],
                [(follower, follow(target, translate([0., 1., 0.])))],
            )
            .unwrap();
        close(
            snapshot.node(follower).unwrap().world,
            translate([time as f32, 1., 0.]),
        );
        snapshots.push(snapshot);
    }
    let held = snapshots[0].node(follower).unwrap().world;
    let parent_world = snapshots[0].node(parent).unwrap().world;
    let released_local = parent_world.inverse().compose(held).unwrap();
    let preserved = graph
        .evaluate_with_transforms([(follower, released_local)])
        .unwrap();
    close(preserved.node(follower).unwrap().world, held);
    close(
        graph
            .evaluate_with_constraints([], [])
            .unwrap()
            .node(follower)
            .unwrap()
            .world,
        translate([12., 0., 0.]),
    );
    close(snapshots[0].node(follower).unwrap().world, held);
    close(snapshots[4].node(follower).unwrap().world, held);
}

#[test]
fn constraints_report_invalid_references_duplicates_and_dependency_cycles() {
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new()).unwrap();
    let b = graph.insert(None, Node::new()).unwrap();
    let child = graph.insert(Some(a), Node::new()).unwrap();
    let stale = graph.insert(None, Node::new()).unwrap();
    graph.remove_subtree(stale).unwrap();
    let mut other = SceneGraph::new();
    let foreign = other.insert(None, Node::new()).unwrap();
    for target in [stale, foreign] {
        assert!(
            matches!(graph.evaluate_with_constraints([], [(a, follow(target, AffineTransform::IDENTITY))]),
            Err(SceneError::InvalidConstraintTarget { node, target: actual }) if node == a && actual == target)
        );
        assert!(
            matches!(graph.evaluate_with_constraints([], [(target, follow(a, AffineTransform::IDENTITY))]),
            Err(SceneError::InvalidHandle(actual)) if actual == target)
        );
    }
    let binding = (a, follow(b, AffineTransform::IDENTITY));
    assert!(
        matches!(graph.evaluate_with_constraints([], [binding, binding]), Err(SceneError::DuplicateConstraint(node)) if node == a)
    );
    assert!(
        matches!(graph.evaluate_with_constraints([(a, AffineTransform::IDENTITY); 2], [binding]), Err(SceneError::DuplicateTransform(node)) if node == a)
    );
    for bindings in [
        vec![(a, follow(a, AffineTransform::IDENTITY))],
        vec![
            (a, follow(b, AffineTransform::IDENTITY)),
            (b, follow(a, AffineTransform::IDENTITY)),
        ],
        vec![(a, follow(child, AffineTransform::IDENTITY))],
        vec![
            (a, follow(b, AffineTransform::IDENTITY)),
            (b, follow(child, AffineTransform::IDENTITY)),
        ],
    ] {
        let error = graph
            .evaluate_with_constraints([], bindings.clone())
            .err()
            .expect("cycle accepted");
        let SceneError::ConstraintCycle(path) = error else {
            panic!("unexpected error: {error}");
        };
        assert!(path.len() >= 2);
        assert_eq!(path.first(), path.last());
        for edge in path.windows(2) {
            let dependency = bindings
                .iter()
                .find_map(|(node, constraint)| match constraint {
                    TransformConstraint::Follow { target, .. } => {
                        (*node == edge[0]).then_some(*target)
                    }
                    _ => None,
                });
            assert!(graph.parent(edge[0]).unwrap() == Some(edge[1]) || dependency == Some(edge[1]));
        }
    }
    assert_eq!(graph.evaluate().unwrap().nodes().len(), 3);
    let valid = graph
        .evaluate_with_constraints(
            [(a, translate([2., 0., 0.]))],
            [(child, follow(a, translate([0., 3., 0.])))],
        )
        .unwrap();
    close(valid.node(child).unwrap().world, translate([2., 3., 0.]));
}

#[test]
fn constrained_composition_reports_overflow_without_modifying_authored_state() {
    let mut graph = SceneGraph::new();
    let source = graph
        .insert(None, Node::new().transform(translate([f32::MAX, 0., 0.])))
        .unwrap();
    let node = graph.insert(None, Node::new()).unwrap();
    let revision = graph.revision();
    assert!(
        matches!(graph.evaluate_with_constraints([], [(node, follow(source, translate([f32::MAX, 0., 0.]))) ]),
        Err(SceneError::InvalidTransform { node: failed, .. }) if failed == node)
    );
    assert_eq!(graph.revision(), revision);
    assert_eq!(
        graph.node(node).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
    assert!(graph.evaluate().is_ok());
}

#[test]
fn long_follow_chains_evaluate_without_recursive_stack_growth() {
    let mut graph = SceneGraph::new();
    let nodes = (0..4096)
        .map(|_| graph.insert(None, Node::new()).unwrap())
        .collect::<Vec<_>>();
    let bindings = nodes
        .windows(2)
        .map(|pair| (pair[1], follow(pair[0], translate([1., 0., 0.]))));
    let result = graph.evaluate_with_constraints([], bindings).unwrap();
    close(
        result.node(nodes[4095]).unwrap().world,
        translate([4095., 0., 0.]),
    );
    close(
        result.node(nodes[0]).unwrap().world,
        AffineTransform::IDENTITY,
    );
}

fn direction(transform: AffineTransform, local: [f32; 3]) -> [f32; 3] {
    let m = transform.matrix();
    std::array::from_fn(|r| (0..3).map(|c| m[c][r] * local[c]).sum())
}

fn unit(value: [f32; 3]) -> [f32; 3] {
    let length = value.iter().map(|value| value * value).sum::<f32>().sqrt();
    value.map(|value| value / length)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

fn vector_close(actual: [f32; 3], expected: [f32; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 2e-5, "{actual} != {expected}");
    }
}

#[test]
fn aim_aligns_custom_axes_and_preserves_affine_shape_and_handedness() {
    let shear = AffineTransform::from_matrix([
        [-2., 0.3, 0., 0.],
        [0.5, 1., 0.2, 0.],
        [0.1, 0.3, 0.5, 0.],
        [2., -1., 3., 1.],
    ])
    .unwrap();
    for transform in [
        AffineTransform::IDENTITY,
        shear,
        AffineTransform::from_trs([1., 2., -3.], [0.2, 0.5, 0.3, 0.6], [0.5, 2., 3.]).unwrap(),
    ] {
        for axis in 0..3 {
            for sign in [-1., 1.] {
                let mut forward = [0.; 3];
                let mut up = [0.; 3];
                forward[axis] = sign;
                up[(axis + 1) % 3] = 1.;
                for toward in [[1., 2., -3.], [0., 0., 1.], [-3., 1., 2.]] {
                    let settings = AimSettings {
                        local_forward: forward,
                        local_up: up,
                        world_up: [0.1, 1., 0.25],
                        ..Default::default()
                    };
                    let origin = transform.transform_point([0.; 3]);
                    let target = std::array::from_fn(|i| origin[i] + toward[i]);
                    let result = settings.solve(transform, target).unwrap();
                    assert!(!result.status.limited);
                    assert_eq!(result.transform.matrix()[3], transform.matrix()[3]);
                    let actual_forward = unit(direction(result.transform, forward));
                    vector_close(actual_forward, unit(toward));
                    let projected_up = |value: [f32; 3]| {
                        unit(std::array::from_fn(|i| {
                            value[i] - actual_forward[i] * dot(value, actual_forward)
                        }))
                    };
                    vector_close(
                        projected_up(direction(result.transform, up)),
                        projected_up(settings.world_up),
                    );
                    let before = transform.matrix();
                    let after = result.transform.matrix();
                    for a in 0..3 {
                        for b in 0..3 {
                            let metric =
                                |m: [[f32; 4]; 4]| (0..3).map(|r| m[a][r] * m[b][r]).sum::<f32>();
                            assert!((metric(before) - metric(after)).abs() < 3e-5);
                        }
                    }
                    let determinant = |m: [[f32; 4]; 4]| {
                        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
                            - m[1][0] * (m[0][1] * m[2][2] - m[0][2] * m[2][1])
                            + m[2][0] * (m[0][1] * m[1][2] - m[0][2] * m[1][1])
                    };
                    assert_eq!(
                        determinant(before).is_sign_positive(),
                        determinant(after).is_sign_positive()
                    );
                }
            }
        }
    }
}

#[test]
fn aim_limits_the_total_correction_including_roll_without_playback_history() {
    use std::f32::consts::{FRAC_1_SQRT_2, FRAC_PI_2, FRAC_PI_4, PI};
    let settings = AimSettings {
        max_angle: FRAC_PI_4,
        ..Default::default()
    };
    let result = settings
        .solve(AffineTransform::IDENTITY, [1., 0., 0.])
        .unwrap();
    assert!(result.status.limited);
    assert!((result.status.requested_angle - FRAC_PI_2).abs() < 1e-6);
    assert_eq!(result.status.applied_angle, FRAC_PI_4);
    vector_close(
        direction(result.transform, [0., 0., -1.]),
        [FRAC_1_SQRT_2, 0., -FRAC_1_SQRT_2],
    );
    let roll = AffineTransform::from_matrix([
        [0., 1., 0., 0.],
        [-1., 0., 0., 0.],
        [0., 0., 1., 0.],
        [0., 0., 0., 1.],
    ])
    .unwrap();
    let rolled = settings.solve(roll, [0., 0., -1.]).unwrap();
    assert!(rolled.status.limited);
    assert!((rolled.status.requested_angle - FRAC_PI_2).abs() < 1e-6);
    vector_close(
        direction(rolled.transform, [0., 1., 0.]),
        [-FRAC_1_SQRT_2, FRAC_1_SQRT_2, 0.],
    );
    vector_close(direction(rolled.transform, [0., 0., -1.]), [0., 0., -1.]);
    let half_turn = AimSettings {
        max_angle: FRAC_PI_2,
        ..Default::default()
    }
    .solve(AffineTransform::IDENTITY, [0., 0., 1.])
    .unwrap();
    assert!((half_turn.status.requested_angle - PI).abs() < 1e-6);
    vector_close(direction(half_turn.transform, [0., 0., -1.]), [-1., 0., 0.]);
    for target in [[1., 0., 0.], [-1., 1., -1.], [1., 0., 0.]] {
        let held = AimSettings {
            max_angle: 0.,
            ..Default::default()
        }
        .solve(roll, target)
        .unwrap();
        assert_eq!(held.transform, roll);
        assert!(held.status.limited);
        assert_eq!(held.status.applied_angle, 0.);
    }
    assert_eq!(
        settings
            .solve(AffineTransform::IDENTITY, [1., 0., 0.])
            .unwrap(),
        result
    );
    let aligned = settings
        .solve(AffineTransform::IDENTITY, [0., 0., -1.])
        .unwrap();
    assert_eq!(aligned.transform, AffineTransform::IDENTITY);
    assert!(!aligned.status.limited);
}

#[test]
fn aim_rejects_degenerate_inputs_and_unrepresentable_rotated_shapes() {
    let source = AffineTransform::IDENTITY;
    let settings = AimSettings::default();
    assert_eq!(
        settings.solve(source, [0.; 3]),
        Err(AimError::CoincidentTarget)
    );
    assert_eq!(
        settings.solve(source, [f32::NAN, 0., 0.]),
        Err(AimError::InvalidTarget)
    );
    for limit in [-1., 4., f32::NAN, f32::INFINITY] {
        assert_eq!(
            AimSettings {
                max_angle: limit,
                ..settings
            }
            .solve(source, [0., 0., -1.]),
            Err(AimError::InvalidLimit)
        );
    }
    for axis in [[0.; 3], [f32::INFINITY, 0., 0.], [0., 1., 0.]] {
        assert_eq!(
            AimSettings {
                local_forward: axis,
                ..settings
            }
            .solve(source, [0., 0., -1.]),
            Err(AimError::InvalidAxes)
        );
    }
    for up in [[0.; 3], [f32::NAN, 0., 0.]] {
        assert_eq!(
            AimSettings {
                world_up: up,
                ..settings
            }
            .solve(source, [0., 0., -1.]),
            Err(AimError::InvalidUp)
        );
    }
    for target in [[0., 1., 0.], [1e-7, 1., 0.]] {
        assert_eq!(
            AimSettings {
                max_angle: 0.,
                ..settings
            }
            .solve(source, target),
            Err(AimError::ParallelUp)
        );
    }
    let tiny_axes = AimSettings {
        local_forward: [0., 0., -1e-30],
        local_up: [0., 1e-30, 0.],
        ..settings
    };
    vector_close(
        direction(
            tiny_axes.solve(source, [1., 0., -1.]).unwrap().transform,
            [0., 0., -1.],
        ),
        unit([1., 0., -1.]),
    );
    let large = AffineTransform::from_matrix([
        [f32::MAX, f32::MAX, 0., 0.],
        [0., f32::MAX, 0., 0.],
        [0., 0., f32::MAX, 0.],
        [0., 0., 0., 1.],
    ])
    .unwrap();
    assert_eq!(
        AimSettings {
            local_forward: [1., 0., 0.],
            local_up: [0., 0., 1.],
            world_up: [0., 0., 1.],
            ..settings
        }
        .solve(large, [1., 0., 0.]),
        Err(AimError::Unrepresentable)
    );
}

#[test]
fn graph_aim_uses_final_target_poses_and_retains_limit_status_with_snapshots() {
    let mut graph = SceneGraph::new();
    let parent_pose =
        AffineTransform::from_trs([1., 0., 0.], [0., 0.2, 0., 0.9], [2., 1., 0.5]).unwrap();
    let parent = graph
        .insert(None, Node::new().transform(parent_pose))
        .unwrap();
    let local_pose = translate([0., 0.5, 0.]);
    let camera_node = graph
        .insert(
            Some(parent),
            Node::new().camera(Camera {
                eye: [0.; 3],
                target: [0., 0., -1.],
                ..Default::default()
            }),
        )
        .unwrap();
    let child = graph
        .insert(
            Some(camera_node),
            Node::new().transform(translate([0., 0., -2.])),
        )
        .unwrap();
    let target = graph.insert(None, Node::new()).unwrap();
    let driver = graph.insert(None, Node::new().visible(false)).unwrap();
    let offset = [0.25, 0.2, 0.];
    let driver_pose =
        AffineTransform::from_trs([-2., 1., -3.], [0.2, 0., 0., 0.9], [1., 2., 1.]).unwrap();
    let bindings = [
        (
            camera_node,
            TransformConstraint::Aim {
                target,
                target_offset: offset,
                settings: AimSettings::default(),
            },
        ),
        (target, follow(driver, translate([0.5, 0., 0.]))),
    ];
    let revision = graph.revision();
    let poses = [(driver, driver_pose), (camera_node, local_pose)];
    let result = graph.evaluate_with_constraints(poses, bindings).unwrap();
    let reordered = graph
        .evaluate_with_constraints(poses, bindings.into_iter().rev())
        .unwrap();
    let world = result.node(camera_node).unwrap().world;
    assert_eq!(
        world.matrix()[3],
        parent_pose.compose(local_pose).unwrap().matrix()[3]
    );
    let point = result.node(target).unwrap().world.transform_point(offset);
    let expected = unit(std::array::from_fn(|i| point[i] - world.matrix()[3][i]));
    vector_close(unit(direction(world, [0., 0., -1.])), expected);
    let camera = result.node(camera_node).unwrap().camera.unwrap();
    let scene = result.scene_from_camera(camera_node).unwrap();
    let prepared = scene
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(prepared.frame().camera_position, camera.eye);
    vector_close(
        unit(std::array::from_fn(|i| camera.target[i] - camera.eye[i])),
        expected,
    );
    close(
        result.node(child).unwrap().world,
        world.compose(translate([0., 0., -2.])).unwrap(),
    );
    close(reordered.node(camera_node).unwrap().world, world);
    assert_eq!(
        result.constraint_status(target),
        Some(ConstraintStatus::Follow)
    );
    let Some(ConstraintStatus::Aim(status)) = result.constraint_status(camera_node) else {
        panic!("missing aim status");
    };
    assert!(!status.limited);
    assert_eq!(result.constraint_status(parent), None);
    assert_eq!(
        reordered.constraint_status(camera_node),
        Some(ConstraintStatus::Aim(status))
    );
    let mut limited_bindings = bindings;
    limited_bindings[0].1 = TransformConstraint::Aim {
        target,
        target_offset: offset,
        settings: AimSettings {
            max_angle: 0.01,
            ..Default::default()
        },
    };
    let limited = graph
        .evaluate_with_constraints(poses, limited_bindings)
        .unwrap();
    assert!(
        matches!(limited.constraint_status(camera_node), Some(ConstraintStatus::Aim(status)) if status.limited && status.applied_angle == 0.01)
    );
    assert_ne!(limited.node(camera_node).unwrap().world, world);
    assert_eq!(
        result.constraint_status(camera_node),
        Some(ConstraintStatus::Aim(status))
    );
    assert_eq!(graph.revision(), revision);
    assert_eq!(
        graph.evaluate().unwrap().constraint_status(camera_node),
        None
    );
    assert!(result.node(camera_node).unwrap().visible);
}

#[test]
fn graph_aim_reports_targets_cycles_and_solver_failures_without_mutating_nodes() {
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let target = graph.insert(None, Node::new()).unwrap();
    let aim = |target, target_offset| TransformConstraint::Aim {
        target,
        target_offset,
        settings: AimSettings::default(),
    };
    assert!(
        matches!(graph.evaluate_with_constraints([], [(node, aim(target, [0.; 3]))]),
        Err(SceneError::InvalidAim { node: failed, source: AimError::CoincidentTarget }) if failed == node)
    );
    assert!(
        matches!(graph.evaluate_with_constraints([], [(node, aim(target, [0., 1., 0.]))]),
        Err(SceneError::InvalidAim { node: failed, source: AimError::ParallelUp }) if failed == node)
    );
    assert!(
        matches!(graph.evaluate_with_constraints([], [(node, aim(target, [f32::NAN, 0., 0.]))]),
        Err(SceneError::InvalidAim { node: failed, source: AimError::InvalidTarget }) if failed == node)
    );
    assert!(matches!(
        graph.evaluate_with_constraints([], [(node, aim(node, [1., 0., 0.]))]),
        Err(SceneError::ConstraintCycle(_))
    ));
    let cycle = [
        (node, aim(target, [1., 0., 0.])),
        (target, follow(node, AffineTransform::IDENTITY)),
    ];
    assert!(matches!(
        graph.evaluate_with_constraints([], cycle),
        Err(SceneError::ConstraintCycle(_))
    ));
    let large = translate([f32::MAX, 0., 0.]);
    assert!(
        matches!(graph.evaluate_with_constraints([(target, large)], [(node, aim(target, [f32::MAX, 0., 0.]))]),
        Err(SceneError::InvalidAim { node: failed, source: AimError::Unrepresentable }) if failed == node)
    );
    let large_target = AffineTransform::from_matrix([
        [f32::MAX, 0., 0., 0.],
        [f32::MAX, f32::MAX, 0., 0.],
        [-f32::MAX, 0., f32::MAX, 0.],
        [0., 0., 0., 1.],
    ])
    .unwrap();
    let result = graph
        .evaluate_with_constraints([(target, large_target)], [(node, aim(target, [1.; 3]))])
        .unwrap();
    vector_close(
        unit(direction(result.node(node).unwrap().world, [0., 0., -1.])),
        unit([1.; 3]),
    );
    graph.remove_subtree(target).unwrap();
    assert!(
        matches!(graph.evaluate_with_constraints([], [(node, aim(target, [1., 0., 0.]))]),
        Err(SceneError::InvalidConstraintTarget { node: failed, target: missing }) if failed == node && missing == target)
    );
    assert_eq!(
        graph.node(node).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
}
