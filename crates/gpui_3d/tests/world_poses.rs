use gpui_3d::{
    AffineTransform, Camera, Material, Mesh, Node, Ray, ResolvedTexture, SceneError, SceneGraph,
    Skin, SkinInfluence, TextureState, TransformOverride as Override,
};

fn translation(position: [f32; 3]) -> AffineTransform {
    AffineTransform::from_translation(position).unwrap()
}

#[test]
fn mixed_spaces_use_final_parents_and_ignore_input_order() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let middle = graph
        .insert(Some(root), Node::new().transform(translation([0., 2., 0.])))
        .unwrap();
    let tip = graph.insert(Some(middle), Node::new()).unwrap();
    let leaf = graph
        .insert(Some(tip), Node::new().transform(translation([0., 0., 3.])))
        .unwrap();
    let sibling = graph.insert(Some(root), Node::new()).unwrap();
    let before = graph.evaluate().unwrap();
    let revision = graph.revision();
    let root_pose = AffineTransform::from_matrix([
        [-2., 0., 0., 0.],
        [0.5, 3., 0., 0.],
        [0., 0., 1., 0.],
        [5., 1., 0., 1.],
    ])
    .unwrap();
    let tip_pose = translation([-3., 4., 1.]);
    let inputs = [
        (tip, Override::World(tip_pose)),
        (sibling, Override::Local(translation([1., 0., 0.]))),
        (root, Override::World(root_pose)),
    ];
    let forward = graph.evaluate_with_transform_overrides(inputs).unwrap();
    let reverse = graph
        .evaluate_with_transform_overrides(inputs.into_iter().rev())
        .unwrap();
    for (node, expected) in [
        (root, root_pose),
        (
            middle,
            root_pose.compose(translation([0., 2., 0.])).unwrap(),
        ),
        (tip, tip_pose),
        (leaf, tip_pose.compose(translation([0., 0., 3.])).unwrap()),
        (
            sibling,
            root_pose.compose(translation([1., 0., 0.])).unwrap(),
        ),
    ] {
        assert_eq!(forward.node(node).unwrap().world, expected);
        assert_eq!(reverse.node(node).unwrap().world, expected);
        assert_eq!(
            forward.node(node).unwrap().parent,
            before.node(node).unwrap().parent
        );
    }
    assert_eq!(graph.revision(), revision);
    assert_eq!(before.node(root).unwrap().world, AffineTransform::IDENTITY);
    assert_eq!(
        graph.world_transform(root).unwrap(),
        AffineTransform::IDENTITY
    );
}

#[test]
fn world_pose_skinning_updates_geometry_bounds_and_picking_together() {
    let base = Mesh::plane();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        base.vertices().iter().map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )
    .unwrap();
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let joint = graph.insert(Some(root), Node::new()).unwrap();
    let surface = graph
        .insert(
            None,
            Node::new()
                .mesh(base.clone(), Material::color(gpui::white()))
                .transform(translation([3., 0., 0.])),
        )
        .unwrap();
    let old = graph.evaluate().unwrap();
    let poses = graph
        .evaluate_with_transform_overrides([
            (root, Override::Local(translation([10., 0., 0.]))),
            (joint, Override::World(translation([2., 0., 0.]))),
        ])
        .unwrap();
    let mesh = skin
        .evaluate_world(
            &base,
            poses.node(surface).unwrap().world,
            &[poses.node(joint).unwrap().world],
        )
        .unwrap();
    let final_pose = poses.with_meshes([(surface, mesh)]).unwrap();
    final_pose.prepare_spatial_index_from(&old);
    let scene = final_pose.scene(Camera::default());
    let ray = Ray::new([2., 0., 2.], [0., 0., -1.]).unwrap();
    assert_eq!(scene.raycast(ray).unwrap().node, Some(surface));
    assert!(old.scene(Camera::default()).raycast(ray).is_none());
    let bounds = final_pose.node(surface).unwrap().bounds.unwrap();
    assert_eq!(bounds.min(), [1.5, -0.5, 0.]);
    assert_eq!(bounds.max(), [2.5, 0.5, 0.]);
    let prepared = scene
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(prepared.objects()[0].node, Some(surface));
    assert_eq!(
        prepared.frame().objects[0].model,
        poses.node(surface).unwrap().world.matrix()
    );
}

#[test]
fn world_overrides_preserve_hidden_hierarchy_and_move_camera_nodes() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new().visible(false)).unwrap();
    let child = graph
        .insert(
            Some(root),
            Node::new()
                .mesh(Mesh::cube(), Material::color(gpui::white()))
                .camera(Camera::default()),
        )
        .unwrap();
    let desired = translation([4., 2., 1.]);
    let poses = graph
        .evaluate_with_transform_overrides([(child, Override::World(desired))])
        .unwrap();
    let node = poses.node(child).unwrap();
    assert!(!node.visible);
    assert_eq!(node.world, desired);
    assert_eq!(
        node.camera.unwrap().eye,
        desired.transform_point(Camera::default().eye)
    );
    assert!(poses.bounds().is_none());
    assert!(poses.node(root).unwrap().subtree_bounds.is_some());
    assert!(
        poses
            .scene(Camera::default())
            .prepare(1., None, |_| unreachable!())
            .unwrap()
            .frame()
            .objects
            .is_empty()
    );
}

#[test]
fn invalid_override_handles_leave_graph_and_snapshots_unchanged() {
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let expired = graph.insert(None, Node::new()).unwrap();
    graph.remove_subtree(expired).unwrap();
    let mut other = SceneGraph::new();
    let foreign = other.insert(None, Node::new()).unwrap();
    let snapshot = graph.evaluate().unwrap();
    let revision = graph.revision();
    for handle in [expired, foreign] {
        assert!(matches!(
            graph.evaluate_with_transform_overrides([(handle, Override::World(AffineTransform::IDENTITY))]),
            Err(SceneError::InvalidHandle(value)) if value == handle
        ));
    }
    assert!(matches!(
        graph.evaluate_with_transform_overrides([
            (node, Override::Local(translation([1., 0., 0.]))),
            (node, Override::World(translation([2., 0., 0.]))),
        ]),
        Err(SceneError::DuplicateTransform(value)) if value == node
    ));
    assert_eq!(graph.revision(), revision);
    assert_eq!(
        graph.world_transform(node).unwrap(),
        snapshot.node(node).unwrap().world
    );
    assert_eq!(
        graph
            .evaluate_with_transform_overrides([])
            .unwrap()
            .node(node)
            .unwrap()
            .world,
        snapshot.node(node).unwrap().world
    );
}
