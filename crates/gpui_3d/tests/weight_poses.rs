use gpui_3d::{Node, PoseMask, SceneGraph, WeightPose, WeightPoseError};

#[test]
fn sparse_layers_preserve_order_signed_values_masks_and_inputs() {
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new()).unwrap();
    let b = graph.insert(None, Node::new()).unwrap();
    let c = graph.insert(None, Node::new()).unwrap();
    let base = WeightPose::new([(b, vec![3.]), (a, vec![-1., 2.]), (c, vec![-0.])]).unwrap();
    let target = WeightPose::new([(a, vec![3., 6.]), (b, vec![-1.])]).unwrap();
    let mask = PoseMask::new(0.5, [(a, 1.), (b, 0.25)]).unwrap();
    let mixed = base.blend(&target, 0.5, Some(&mask)).unwrap();
    assert_eq!(
        mixed.weights(),
        &[(b, vec![2.5]), (a, vec![1., 4.]), (c, vec![-0.])]
    );
    assert_eq!(mixed.get(c).unwrap()[0].to_bits(), (-0.0_f32).to_bits());
    let reference = WeightPose::new([(a, vec![1., 2.]), (b, vec![1.])]).unwrap();
    let added = base
        .additive(&target, &reference, 0.5, Some(&mask))
        .unwrap();
    assert_eq!(added.get(a), Some([0., 4.].as_slice()));
    assert_eq!(added.get(b), Some([2.75].as_slice()));
    assert_eq!(base.get(a), Some([-1., 2.].as_slice()));
    assert_eq!(target.get(a), Some([3., 6.].as_slice()));
    assert_eq!(reference.get(a), Some([1., 2.].as_slice()));
    let zero = base.blend(&target, 0., Some(&mask)).unwrap();
    assert!(std::ptr::eq(base.weights(), zero.weights()));
    let reversed = target
        .blend(
            &WeightPose::new([(a, vec![-1., 2.]), (b, vec![3.])]).unwrap(),
            1.,
            None,
        )
        .unwrap();
    assert_eq!(reversed.weights()[0].0, a);
    assert_eq!(base.clone().into_weights(), base.weights());
    let expected = mixed.weights().to_vec();
    assert_eq!(mixed.into_weights(), expected);
}

#[test]
fn invalid_arrays_and_layer_shapes_are_rejected_even_when_masked_out() {
    let mut graph = SceneGraph::new();
    let node = graph.insert(None, Node::new()).unwrap();
    let other = graph.insert(None, Node::new()).unwrap();
    assert!(
        matches!(WeightPose::new([(node, vec![])]), Err(WeightPoseError::EmptyWeights(n)) if n == node)
    );
    assert!(
        matches!(WeightPose::new([(node, vec![1.]), (node, vec![2.])]), Err(WeightPoseError::DuplicateNode(n)) if n == node)
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            matches!(WeightPose::new([(node, vec![0., value])]), Err(WeightPoseError::InvalidValue { node: n, component: 1 }) if n == node)
        );
    }
    let base = WeightPose::new([(node, vec![1., 2.])]).unwrap();
    let short = WeightPose::new([(node, vec![1.])]).unwrap();
    let missing = WeightPose::new([(other, vec![1., 2.])]).unwrap();
    let mask = PoseMask::new(0., []).unwrap();
    for weight in [0., 1.] {
        assert!(
            matches!(base.blend(&short, weight, Some(&mask)), Err(WeightPoseError::WeightCount { node: n, expected: 2, actual: 1 }) if n == node)
        );
        assert!(
            matches!(base.blend(&missing, weight, Some(&mask)), Err(WeightPoseError::MissingNode(n)) if n == other)
        );
        assert!(
            matches!(base.additive(&base, &WeightPose::default(), weight, Some(&mask)), Err(WeightPoseError::MissingReference(n)) if n == node)
        );
        assert!(
            matches!(base.additive(&base, &short, weight, Some(&mask)), Err(WeightPoseError::WeightCount { node: n, .. }) if n == node)
        );
    }
    let invalid_mask = PoseMask::new(0., [(other, 0.)]).unwrap();
    assert!(
        matches!(base.blend(&WeightPose::default(), 0., Some(&invalid_mask)), Err(WeightPoseError::MissingNode(n)) if n == other)
    );
    for weight in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            base.blend(&base, weight, None),
            Err(WeightPoseError::InvalidBlendWeight)
        ));
    }
    assert_eq!(base.get(node), Some([1., 2.].as_slice()));
}

#[test]
fn wide_intermediates_and_late_overflow_preserve_previous_results() {
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new()).unwrap();
    let b = graph.insert(None, Node::new()).unwrap();
    let base = WeightPose::new([(a, vec![0.]), (b, vec![f32::MAX])]).unwrap();
    let target = WeightPose::new([(a, vec![2.]), (b, vec![-f32::MAX])]).unwrap();
    let middle = base.blend(&target, 0.5, None).unwrap();
    assert_eq!(middle.get(a), Some([1.].as_slice()));
    assert_eq!(middle.get(b), Some([0.].as_slice()));
    let negative_zero = WeightPose::new([(a, vec![-0.])]).unwrap();
    assert_eq!(
        base.blend(&negative_zero, 1., None)
            .unwrap()
            .get(a)
            .unwrap()[0]
            .to_bits(),
        (-0.0_f32).to_bits()
    );
    assert!(
        matches!(base.additive(&base, &target, 1., None), Err(WeightPoseError::Unrepresentable { node, component: 0 }) if node == b)
    );
    assert_eq!(middle.get(a), Some([1.].as_slice()));
    assert_eq!(base.get(a), Some([0.].as_slice()));
    assert_eq!(base.get(b), Some([f32::MAX].as_slice()));
    let big = WeightPose::new([(a, vec![1e30])]).unwrap();
    let mask = PoseMask::new(1e-20, []).unwrap();
    let tiny = base.blend(&big, 1e-30, Some(&mask)).unwrap();
    assert!((tiny.get(a).unwrap()[0] - 1e-20).abs() < 1e-25);
}

#[test]
fn mixed_weights_feed_morph_skin_and_final_scene_queries() {
    use gpui::rgb;
    use gpui_3d::{
        AffineTransform, Camera, Material, Mesh, MorphTarget, MorphTargets, Ray, Skin,
        SkinInfluence,
    };
    let mut graph = SceneGraph::new();
    let base = Mesh::plane();
    let node = graph
        .insert(
            None,
            Node::new()
                .mesh(base.clone(), Material::color(rgb(0xffffff)))
                .transform(AffineTransform::from_translation([2., 0., 0.]).unwrap()),
        )
        .unwrap();
    let before = graph.evaluate().unwrap();
    let morph = MorphTargets::new(
        base.clone(),
        [[2., 0., 0.], [0., 0., 4.]].map(|delta| MorphTarget {
            positions: Some(vec![delta; base.vertex_count()].into()),
            ..Default::default()
        }),
    )
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        (0..base.vertex_count()).map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )
    .unwrap();
    let idle = WeightPose::new([(node, vec![0., 0.])]).unwrap();
    let action = WeightPose::new([(node, vec![1., 0.5])]).unwrap();
    let weights = idle.blend(&action, 0.5, None).unwrap();
    let mesh = skin
        .evaluate_world(
            &morph.evaluate(weights.get(node).unwrap()).unwrap(),
            before.node(node).unwrap().world,
            &[AffineTransform::from_translation([0., 2., 0.]).unwrap()],
        )
        .unwrap();
    let sample = graph.evaluate_with_overrides([], [(node, mesh)]).unwrap();
    sample.prepare_spatial_index_from(&before);
    let ray = Ray::new([1., 2., 5.], [0., 0., -1.]).unwrap();
    let hit = sample.scene(Camera::default()).raycast(ray).unwrap();
    assert_eq!(hit.node, Some(node));
    assert_eq!(hit.position, [1., 2., 1.]);
    assert_eq!(
        sample.node(node).unwrap().bounds.unwrap(),
        gpui_3d::Aabb::new([0.5, 1.5, 1.], [1.5, 2.5, 1.]).unwrap()
    );
    assert!(before.scene(Camera::default()).raycast(ray).is_none());
    assert_eq!(graph.evaluate().unwrap().bounds(), before.bounds());
}
