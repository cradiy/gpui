use super::*;
use gpui_3d::{AffineTransform, Node, SceneError};
use gpui_3d_gltf::AnimationTargetPolicy;

#[test]
fn authored_bases_mix_disjoint_clips_without_losing_defaults_or_instance_placement() {
    let mut fixture = Fixture::new();
    fixture.morph_mesh();
    fixture.json["meshes"][0]["weights"] = json!([0.25, 0.5]);
    let primitive = fixture.json["meshes"][0]["primitives"][0].clone();
    fixture.json["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(primitive);
    fixture.json["nodes"][1]["weights"] = json!([0.5, -0.25]);
    fixture.json["nodes"][0]["children"] = json!([1, 2, 3]);
    fixture.json["nodes"].as_array_mut().unwrap().extend([
        json!({"mesh":0,"translation":[-2,0,0]}),
        json!({"matrix":[1,0,0,0, 0.5,1,0,0, 0,0,1,0, 0,4,0,1]}),
    ]);
    let times = fixture.times(&[0., 2.]);
    let translation = fixture.floats("VEC3", &[1., 2., 3., 5., 2., 3.]);
    let weights = fixture.floats("SCALAR", &[0.5, -0.25, 1., 0.]);
    fixture.channel(1, "translation", times, translation, "LINEAR");
    fixture.channel(1, "weights", times, weights, "LINEAR");
    let first_clip = fixture.json["animations"][0].clone();
    fixture.json["animations"][0] = json!({"channels":[],"samplers":[]});
    let translation = fixture.floats("VEC3", &[-2., 0., 0., -6., 0., 0.]);
    let weights = fixture.floats("SCALAR", &[0.25, 0.5, -0.25, 0.5]);
    fixture.channel(2, "translation", times, translation, "LINEAR");
    fixture.channel(2, "weights", times, weights, "LINEAR");
    let second_clip = fixture.json["animations"][0].clone();
    fixture.json["animations"] = json!([first_clip, second_clip]);
    let document = fixture.prepare().unwrap();
    let asset = document
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    let mut graph = SceneGraph::new();
    let a = asset.instantiate(&mut graph, None).unwrap();
    let b = asset.instantiate(&mut graph, None).unwrap();
    let first = document
        .animation(0, AnimationOptions::default())
        .unwrap()
        .bind(&a, AnimationTargetPolicy::RequireAll)
        .unwrap();
    let second = document
        .animation(1, AnimationOptions::default())
        .unwrap()
        .bind(&a, AnimationTargetPolicy::RequireAll)
        .unwrap();
    drop(asset);
    drop(document);
    let pose = a.authored_pose().unwrap();
    let weights = a.authored_weights().unwrap();
    let n1 = a.node(1).unwrap();
    let n2 = a.node(2).unwrap();
    assert_eq!(pose.len(), 3);
    assert!(pose.get(a.root()).is_none());
    assert!(pose.get(a.node(3).unwrap()).is_none());
    assert!(pose.get(a.primitive(1, 0).unwrap()).is_none());
    assert_eq!(
        weights.weights(),
        &[(n1, vec![0.5, -0.25]), (n2, vec![0.25, 0.5])]
    );
    assert!(b.authored_pose().unwrap().get(n1).is_none());
    graph
        .set_transform(
            a.root(),
            AffineTransform::from_translation([20., 0., 0.]).unwrap(),
        )
        .unwrap();
    let untouched = graph.evaluate().unwrap();
    graph
        .set_transform(
            n1,
            AffineTransform::from_translation([100., 0., 0.]).unwrap(),
        )
        .unwrap();
    assert_eq!(a.authored_pose().unwrap().get(n1), pose.get(n1));
    let revision = graph.revision();
    for seconds in [2, 0, 1, 2] {
        let time = Duration::from_secs(seconds);
        let first = first.sample(time).unwrap();
        let second = second.sample(time).unwrap();
        let mixed_pose = pose
            .blend(first.pose(), 1., None)
            .unwrap()
            .blend(&pose.blend(second.pose(), 1., None).unwrap(), 0.5, None)
            .unwrap();
        let mixed_weights = weights
            .blend(first.weight_pose(), 1., None)
            .unwrap()
            .blend(
                &weights.blend(second.weight_pose(), 1., None).unwrap(),
                0.5,
                None,
            )
            .unwrap();
        let poses = graph
            .evaluate_with_transforms(mixed_pose.transforms())
            .unwrap();
        let meshes = a.deform(&poses, mixed_weights.weights()).unwrap();
        let evaluated = poses.with_meshes(meshes).unwrap();
        let t = seconds as f32;
        near(
            evaluated.node(n1).unwrap().world.transform_point([0.; 3]),
            [31. + t, 2., 3.],
        );
        near(
            evaluated.node(n2).unwrap().world.transform_point([0.; 3]),
            [28. - t, 0., 0.],
        );
        assert_eq!(
            evaluated.node(a.primitive(1, 0).unwrap()).unwrap().bounds,
            Some(
                gpui_3d::Aabb::new([31. + t, 2., 3.], [33.5 + 1.375 * t, 5.75 + 0.5625 * t, 3.])
                    .unwrap()
            )
        );
        for node in [a.node(3).unwrap(), b.node(1).unwrap(), b.node(2).unwrap()] {
            assert_eq!(
                evaluated.node(node).unwrap().world,
                untouched.node(node).unwrap().world
            );
        }
        let hit = evaluated
            .scene(gpui_3d::Camera::default())
            .raycast(gpui_3d::Ray::new([31.1 + t, 2.1, 10.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(a.source_primitive(hit.node.unwrap()).unwrap().node_index, 1);
        assert_eq!(graph.revision(), revision);
    }
    assert_eq!(weights.get(n1), Some([0.5, -0.25].as_slice()));
    graph.remove_subtree(a.root()).unwrap();
    assert!(
        graph
            .evaluate_with_transforms(a.authored_pose().unwrap().transforms())
            .is_err()
    );
}

#[test]
fn authored_bases_preserve_signed_trs_and_zero_morph_defaults_without_clips() {
    let mut fixture = Fixture::new();
    fixture.morph_mesh();
    fixture.json.as_object_mut().unwrap().remove("animations");
    fixture.json["meshes"][0]
        .as_object_mut()
        .unwrap()
        .remove("weights");
    fixture.json["nodes"][1]["scale"] = json!([-2, 3, 4]);
    fixture.json["nodes"][1]["rotation"] = json!([0, 0, 0, -1]);
    let asset = fixture
        .prepare()
        .unwrap()
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    let mut graph = SceneGraph::new();
    let instance = asset.instantiate(&mut graph, None).unwrap();
    let pose = instance.authored_pose().unwrap();
    let weights = instance.authored_weights().unwrap();
    let node = instance.node(1).unwrap();
    assert_eq!(pose.get(node).unwrap().scale, [-2., 3., 4.]);
    assert_eq!(pose.get(node).unwrap().rotation, [0., 0., 0., -1.]);
    assert_eq!(weights.get(node), Some([0., 0.].as_slice()));
    let original = graph.evaluate().unwrap();
    let poses = graph.evaluate_with_transforms(pose.transforms()).unwrap();
    let meshes = instance.deform(&poses, weights.weights()).unwrap();
    let evaluated = graph
        .evaluate_with_overrides(pose.transforms(), meshes)
        .unwrap();
    assert_eq!(evaluated.bounds(), original.bounds());
    for node in original.nodes() {
        let unchanged = evaluated.node(node.handle).unwrap();
        assert_eq!(unchanged.world, node.world);
        assert_eq!(unchanged.bounds, node.bounds);
    }
}

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
    let start = bound_a.sample(Duration::ZERO).unwrap();
    let end = bound_a.sample(Duration::from_secs(2)).unwrap();
    let middle = bound_a.sample(Duration::from_secs(1)).unwrap();
    let mixed = start
        .weight_pose()
        .blend(end.weight_pose(), 0.5, None)
        .unwrap();
    assert_eq!(mixed.weights(), middle.weights());
    let poses = graph
        .evaluate_with_transforms(middle.pose().transforms())
        .unwrap();
    let meshes = a.deform(&poses, mixed.weights()).unwrap();
    let evaluated = graph
        .evaluate_with_overrides(middle.pose().transforms(), meshes)
        .unwrap();
    assert_eq!(
        evaluated.node(a.primitive(1, 0).unwrap()).unwrap().bounds,
        Some(gpui_3d::Aabb::new([12., 1., 0.], [14.5, 4.75, 0.]).unwrap())
    );
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
