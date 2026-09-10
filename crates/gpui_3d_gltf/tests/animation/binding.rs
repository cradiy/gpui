use super::*;
use gpui_3d::{AffineTransform, Node, SceneError};
use gpui_3d_gltf::AnimationTargetPolicy;

#[test]
fn bound_samples_retain_tracks_and_keep_instance_poses_and_weights_independent() {
    let mut fixture = Fixture::new();
    fixture.morph_mesh();
    let times = fixture.times(&[0., 2.]);
    let translation = fixture.floats("VEC3", &[0., 1., 0., 4., 1., 0.]);
    let weights = fixture.floats("SCALAR", &[0., 0., 1., -0.5]);
    fixture.channel(1, "translation", times, translation, "LINEAR");
    fixture.channel(1, "weights", times, weights, "LINEAR");
    let prepared = fixture.prepare().unwrap();
    let clip = prepared.animation(0, AnimationOptions::default()).unwrap();
    let scene_source = prepared.clone();
    drop(prepared);
    let asset = scene_source
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    let mut graph = SceneGraph::new();
    let a = asset.instantiate(&mut graph, None).unwrap();
    let parent = graph
        .insert(
            None,
            Node::new().transform(AffineTransform::from_translation([20., 0., 0.]).unwrap()),
        )
        .unwrap();
    let b = asset.clone().instantiate(&mut graph, Some(parent)).unwrap();
    let bound_a = clip.bind(&a, AnimationTargetPolicy::RequireAll).unwrap();
    let bound_b = clip.bind(&b, AnimationTargetPolicy::RequireAll).unwrap();
    drop(scene_source);
    drop(clip);
    drop(asset);
    let node_a = a.node(1).unwrap();
    let node_b = b.node(1).unwrap();
    let sample_b = std::thread::spawn(move || bound_b.sample(Duration::from_secs(2)).unwrap())
        .join()
        .unwrap();
    for time in [1, 0, 2, 1] {
        let sample_a = bound_a.clone().sample(Duration::from_secs(time)).unwrap();
        near(
            sample_a.pose().get(node_a).unwrap().translation,
            [2. * time as f32, 1., 0.],
        );
        near(sample_a.pose().get(node_a).unwrap().scale, [2., 3., 4.]);
        assert!(sample_a.pose().get(node_b).is_none());
        assert_eq!(
            sample_a.weights(),
            &[(node_a, vec![time as f32 * 0.5, time as f32 * -0.25])]
        );
        let poses = graph
            .evaluate_with_transforms(
                sample_a
                    .pose()
                    .transforms()
                    .chain(sample_b.pose().transforms()),
            )
            .unwrap();
        let meshes_a = a.deform(&poses, sample_a.weights()).unwrap();
        let meshes_b = b.deform(&poses, sample_b.weights()).unwrap();
        let evaluated = graph
            .evaluate_with_overrides(
                sample_a
                    .pose()
                    .transforms()
                    .chain(sample_b.pose().transforms()),
                meshes_a.into_iter().chain(meshes_b),
            )
            .unwrap();
        near(
            evaluated
                .node(node_a)
                .unwrap()
                .world
                .transform_point([0.; 3]),
            [10. + 2. * time as f32, 1., 0.],
        );
        near(
            evaluated
                .node(node_b)
                .unwrap()
                .world
                .transform_point([0.; 3]),
            [34., 1., 0.],
        );
        near(
            sample_b.pose().get(node_b).unwrap().translation,
            [4., 1., 0.],
        );
    }
    let sample = bound_a.sample(Duration::ZERO).unwrap();
    graph.remove_subtree(a.root()).unwrap();
    graph.insert(None, Node::new()).unwrap();
    assert!(
        matches!(graph.evaluate_with_transforms(sample.pose().transforms()), Err(SceneError::InvalidHandle(n)) if n == node_a)
    );
}

#[test]
fn binding_requires_source_identity_and_explicit_missing_target_policy() {
    let mut fixture = Fixture::new();
    fixture.json["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({}));
    fixture.json["scenes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"nodes":[]}));
    let times = fixture.times(&[0., 2.]);
    let values = fixture.floats("VEC3", &[0., 0., 0., 2., 0., 0.]);
    fixture.channel(2, "translation", times, values, "LINEAR");
    fixture.channel(1, "translation", times, values, "LINEAR");
    let prepared = fixture.prepare().unwrap();
    let clip = prepared.animation(0, AnimationOptions::default()).unwrap();
    let asset = prepared
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    let mut graph = SceneGraph::new();
    let instance = asset.instantiate(&mut graph, None).unwrap();
    let error = clip
        .bind(&instance, AnimationTargetPolicy::RequireAll)
        .unwrap_err();
    assert!(error.to_string().contains("target node 2"));
    let bound = clip
        .bind(&instance, AnimationTargetPolicy::SkipMissing)
        .unwrap();
    assert_eq!(bound.missing_nodes(), &[2]);
    assert_eq!(
        bound.sample(Duration::from_secs(1)).unwrap().pose().len(),
        1
    );
    let empty = prepared
        .scene(Some(1), SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap()
        .instantiate(&mut graph, None)
        .unwrap();
    let empty = clip
        .bind(&empty, AnimationTargetPolicy::SkipMissing)
        .unwrap();
    assert_eq!(empty.missing_nodes(), &[2, 1]);
    let sample = empty.sample(Duration::from_secs(1)).unwrap();
    assert!(sample.pose().is_empty() && sample.weights().is_empty());
    let other = fixture
        .prepare()
        .unwrap()
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap()
        .instantiate(&mut graph, None)
        .unwrap();
    for policy in [
        AnimationTargetPolicy::RequireAll,
        AnimationTargetPolicy::SkipMissing,
    ] {
        assert!(
            clip.bind(&other, policy)
                .unwrap_err()
                .to_string()
                .contains("different documents")
        );
    }
}

#[test]
fn failed_bound_samples_preserve_previous_results_and_can_be_retried() {
    let mut fixture = Fixture::new();
    let times = fixture.times(&[0., 2.]);
    let translation = fixture.floats("VEC3", &[0., 0., 0., 4., 0., 0.]);
    let scale = fixture.floats("VEC3", &[1., 1., 1., -1., 1., 1.]);
    fixture.channel(0, "translation", times, translation, "LINEAR");
    fixture.channel(1, "scale", times, scale, "LINEAR");
    let prepared = fixture.prepare().unwrap();
    let clip = prepared.animation(0, AnimationOptions::default()).unwrap();
    let asset = prepared
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    let mut graph = SceneGraph::new();
    let instance = asset.instantiate(&mut graph, None).unwrap();
    let bound = clip
        .bind(&instance, AnimationTargetPolicy::RequireAll)
        .unwrap();
    let old = bound.sample(Duration::ZERO).unwrap();
    let node = instance.node(1).unwrap();
    let error = bound.sample(Duration::from_secs(1)).unwrap_err();
    assert!(error.to_string().contains("animation 0 node 1"));
    near(old.pose().get(node).unwrap().scale, [1., 1., 1.]);
    near(
        bound
            .sample(Duration::from_secs(2))
            .unwrap()
            .pose()
            .get(node)
            .unwrap()
            .scale,
        [-1., 1., 1.],
    );
    near(
        bound
            .sample(Duration::ZERO)
            .unwrap()
            .pose()
            .get(node)
            .unwrap()
            .translation,
        [1., 2., 3.],
    );
}
