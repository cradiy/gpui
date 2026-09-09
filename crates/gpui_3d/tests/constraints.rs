use gpui::rgb;
use gpui_3d::{
    AffineTransform, Camera, Interpolation, Keyframe, Material, Mesh, Node, NodeHandle, Ray,
    ResolvedTexture, SceneError, SceneGraph, TextureState, TransformConstraint, TransformPose,
    TransformTrack, VectorTrack,
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
            let dependency = bindings.iter().find_map(|(node, constraint)| {
                let TransformConstraint::Follow { target, .. } = constraint;
                (*node == edge[0]).then_some(*target)
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
