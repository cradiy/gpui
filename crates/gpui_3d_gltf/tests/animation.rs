use std::time::Duration;

use gpui_3d::{Pose, SceneGraph};
use gpui_3d_gltf::{
    AnimationOptions, Document, ImageDecodeLimits, Limits, PreparedDocument, SceneOptions,
};
use serde_json::{Value, json};

#[path = "animation/binding.rs"]
mod binding;

struct Fixture {
    json: Value,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            json: json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"translation":[10,0,0],"children":[1]}, {"translation":[1,2,3],"scale":[2,3,4]}],
                "accessors":[],"bufferViews":[],"animations":[{"name":"move","channels":[],"samplers":[]}]}),
            bytes: Vec::new(),
        }
    }

    fn view(&mut self, bytes: &[u8]) -> usize {
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        let offset = self.bytes.len();
        self.bytes.extend(bytes);
        let views = self.json["bufferViews"].as_array_mut().unwrap();
        let index = views.len();
        views.push(json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()}));
        index
    }

    fn accessor(&mut self, accessor: Value) -> usize {
        let list = self.json["accessors"].as_array_mut().unwrap();
        let index = list.len();
        list.push(accessor);
        index
    }

    fn floats(&mut self, kind: &str, values: &[f32]) -> usize {
        let bytes: Vec<_> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let view = self.view(&bytes);
        let components = match kind {
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 1,
        };
        self.accessor(json!({"bufferView":view,"componentType":5126,"type":kind,"count":values.len()/components}))
    }

    fn times(&mut self, values: &[f32]) -> usize {
        let index = self.floats("SCALAR", values);
        self.json["accessors"][index]["min"] = json!([0.]);
        self.json["accessors"][index]["max"] = json!([10.]);
        index
    }

    fn channel(
        &mut self,
        node: usize,
        path: &str,
        input: usize,
        output: usize,
        interpolation: &str,
    ) {
        let animation = &mut self.json["animations"][0];
        let sampler = animation["samplers"].as_array().unwrap().len();
        animation["samplers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"input":input,"output":output,"interpolation":interpolation}));
        animation["channels"]
            .as_array_mut()
            .unwrap()
            .push(json!({"sampler":sampler,"target":{"node":node,"path":path}}));
    }

    fn prepare(&self) -> anyhow::Result<PreparedDocument> {
        let mut source = self.json.clone();
        source["buffers"] = json!([{"uri":"animation.bin","byteLength":self.bytes.len()}]);
        Document::from_slice(&serde_json::to_vec(&source)?, Limits::default())?
            .prepare(|_| Ok(self.bytes.clone()))
    }

    fn morph_mesh(&mut self) {
        let positions = self.floats("VEC3", &[0., 0., 0., 1., 0., 0., 0., 1., 0.]);
        self.json["accessors"][positions]["min"] = json!([0., 0., 0.]);
        self.json["accessors"][positions]["max"] = json!([1., 1., 0.]);
        self.json["meshes"] = json!([{"weights":[0.,0.],"primitives":[{"attributes":{"POSITION":positions},
            "targets":[{"POSITION":positions},{"POSITION":positions}]}]}]);
        self.json["nodes"][1]["mesh"] = json!(0);
    }
}

fn near<const N: usize>(actual: [f32; N], expected: [f32; N]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }
}

#[test]
fn playback_maps_relative_controls_to_authored_times_in_both_directions() {
    let mut fixture = Fixture::new();
    let input = fixture.times(&[2., 4.]);
    let output = fixture.floats("VEC3", &[0., 0., 0., 4., 0., 0.]);
    fixture.channel(1, "translation", input, output, "LINEAR");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    let mut playback = gpui_3d_gltf::AnimationPlayback::new(&clip);
    assert_eq!(playback.time(), Duration::from_secs(2));
    assert!(!playback.advance(Duration::from_secs(9)).unwrap());
    playback.play();
    playback.advance(Duration::from_millis(500)).unwrap();
    near(
        clip.nodes()[0]
            .transform()
            .unwrap()
            .sample(playback.time())
            .unwrap()
            .translation,
        [1., 0., 0.],
    );
    playback.set_rate(2.).unwrap();
    playback.advance(Duration::from_millis(250)).unwrap();
    assert_eq!(playback.position(), Duration::from_secs(1));
    let independent = playback.clone();
    playback.seek(Duration::MAX);
    assert_eq!(playback.time(), clip.end());
    assert!(!playback.is_playing());
    assert_eq!(independent.position(), Duration::from_secs(1));
    playback.play();
    assert_eq!(playback.position(), Duration::ZERO);
    playback.advance(Duration::from_secs(5)).unwrap();
    assert_eq!(playback.time(), clip.end());
    assert!(!playback.is_playing());
    playback.set_rate(-1.).unwrap();
    playback.set_looping(true);
    playback.play();
    playback.advance(Duration::from_millis(2500)).unwrap();
    assert_eq!(playback.position(), Duration::from_millis(1500));
    playback.set_looping(false);
    playback.advance(Duration::from_secs(2)).unwrap();
    assert_eq!(playback.time(), clip.start());
    assert!(!playback.is_playing());
    playback.play();
    assert_eq!(playback.time(), clip.end());
    playback.set_rate(0.5).unwrap();
    playback.seek(Duration::ZERO);
    playback.set_looping(true);
    playback.play();
    playback.advance(Duration::from_secs(4)).unwrap();
    assert_eq!(playback.time(), clip.start());
    assert!(playback.is_playing());
}

#[test]
fn playback_is_partition_independent_and_rejects_overflow_without_losing_position() {
    let mut fixture = Fixture::new();
    let input = fixture.times(&[2., 4.]);
    let output = fixture.floats("VEC3", &[0., 0., 0., 4., 0., 0.]);
    fixture.channel(1, "translation", input, output, "LINEAR");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    let mut split = gpui_3d_gltf::AnimationPlayback::new(&clip);
    split.set_rate(0.3).unwrap();
    split.play();
    let mut whole = split.clone();
    for _ in 0..10 {
        split.advance(Duration::from_nanos(1)).unwrap();
    }
    whole.advance(Duration::from_nanos(10)).unwrap();
    assert_eq!(split.position(), whole.position());
    assert_eq!(split.position(), Duration::from_nanos(3));
    for rate in [0., f64::NAN, f64::INFINITY] {
        assert!(split.set_rate(rate).is_err());
        assert_eq!(split.rate(), 0.3);
    }
    split.set_rate(f64::MAX).unwrap();
    assert!(split.advance(Duration::from_secs(2)).is_err());
    assert_eq!(split.position(), Duration::from_nanos(3));
    assert!(split.is_playing());
    split.pause();
    assert!(!split.advance(Duration::MAX).unwrap());

    let mut single = Fixture::new();
    let input = single.times(&[7.]);
    let output = single.floats("VEC3", &[1., 2., 3.]);
    single.channel(1, "translation", input, output, "STEP");
    let clip = single
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    let mut playback = gpui_3d_gltf::AnimationPlayback::new(&clip);
    playback.set_looping(true);
    playback.play();
    assert!(!playback.is_playing());
    assert!(!playback.advance(Duration::MAX).unwrap());
    assert_eq!(playback.time(), Duration::from_secs(7));
}

#[test]
fn transform_clips_preserve_base_channels_and_independent_instance_random_access() {
    let mut fixture = Fixture::new();
    let input = fixture.times(&[1., 3.]);
    let output = fixture.floats("VEC3", &[0., 2., 3., 4., 2., 3.]);
    fixture.channel(1, "translation", input, output, "LINEAR");
    let scale = fixture.floats("VEC3", &[2., 3., 4., 4., 3., 4.]);
    fixture.channel(1, "scale", input, scale, "STEP");
    let input = fixture.times(&[0., 4.]);
    let parent = fixture.floats("VEC3", &[10., 0., 0., 14., 0., 0.]);
    fixture.channel(0, "translation", input, parent, "LINEAR");
    let document = fixture.prepare().unwrap();
    let clip = std::thread::spawn({
        let document = document.clone();
        move || document.animation(0, AnimationOptions::default()).unwrap()
    })
    .join()
    .unwrap();
    assert_eq!(
        clip.nodes()
            .iter()
            .map(|node| node.node_index())
            .collect::<Vec<_>>(),
        [1, 0]
    );
    assert_eq!(clip.name(), Some("move"));
    assert_eq!(
        (clip.start(), clip.end(), clip.duration()),
        (
            Duration::ZERO,
            Duration::from_secs(4),
            Duration::from_secs(4)
        )
    );
    let asset = document
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    drop(document);
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree()).unwrap();
    let second = graph.instantiate(None, asset.subtree()).unwrap();
    let source = |index| {
        asset
            .nodes()
            .iter()
            .find(|node| node.index == index)
            .unwrap()
            .handle
    };
    let mut retained = None;
    for second_time in [0., 4., 2., 3., 2.] {
        let mut locals = Vec::new();
        for (instance, time) in [(&first, 2.), (&second, second_time)] {
            for node in clip.nodes() {
                let track = node.transform().unwrap();
                locals.push((
                    instance.node(source(node.node_index())).unwrap(),
                    track.sample(Duration::from_secs_f32(time)).unwrap(),
                ));
            }
        }
        let pose = Pose::new(locals).unwrap();
        let evaluated = graph.evaluate_with_transforms(pose.transforms()).unwrap();
        let first_node = first.node(source(1)).unwrap();
        let second_node = second.node(source(1)).unwrap();
        near(
            evaluated
                .node(first_node)
                .unwrap()
                .world
                .transform_point([1., 0., 0.]),
            [16., 2., 3.],
        );
        let x = 10.
            + second_time
            + (2. * (second_time - 1.)).clamp(0., 4.)
            + if second_time < 3. { 2. } else { 4. };
        near(
            evaluated
                .node(second_node)
                .unwrap()
                .world
                .transform_point([1., 0., 0.]),
            [x, 2., 3.],
        );
        retained.get_or_insert(evaluated);
    }
    near(
        retained
            .unwrap()
            .node(second.node(source(1)).unwrap())
            .unwrap()
            .world
            .transform_point([1., 0., 0.]),
        [12., 2., 3.],
    );
    near(
        graph
            .evaluate()
            .unwrap()
            .node(first.node(source(1)).unwrap())
            .unwrap()
            .world
            .transform_point([0.; 3]),
        [11., 2., 3.],
    );
}

#[test]
fn cubic_vectors_and_quaternions_preserve_per_second_tangents() {
    let mut fixture = Fixture::new();
    let input = fixture.times(&[2., 4.]);
    let output = fixture.floats(
        "VEC3",
        &[
            0., 0., 0., 0., 0., 0., 2., 0., 0., 0., 0., 0., 2., 0., 0., 0., 0., 0.,
        ],
    );
    fixture.channel(1, "translation", input, output, "CUBICSPLINE");
    let rotation = fixture.floats(
        "VEC4",
        &[
            0., 0., 0., 0., 0., 0., 0., 1., 0., 0., 1., 0., 0., 0., 0., 0., 0., 0., 1., 0., 0., 0.,
            0., 0.,
        ],
    );
    fixture.channel(1, "rotation", input, rotation, "CUBICSPLINE");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    assert_eq!(
        (clip.start(), clip.end(), clip.duration()),
        (
            Duration::from_secs(2),
            Duration::from_secs(4),
            Duration::from_secs(2)
        )
    );
    let track = clip.nodes()[0].transform().unwrap();
    let pose = track.sample(Duration::from_secs(3)).unwrap();
    near(pose.translation, [1.5, 0., 0.]);
    near(
        pose.rotation,
        [0., 0., 3. / 13_f32.sqrt(), 2. / 13_f32.sqrt()],
    );
    near(pose.scale, [2., 3., 4.]);
    near(track.sample(Duration::ZERO).unwrap().translation, [0.; 3]);
    near(
        track.sample(Duration::from_secs(10)).unwrap().translation,
        [2., 0., 0.],
    );
}

#[test]
fn normalized_rotations_and_signed_weight_channels_keep_their_sampling_semantics() {
    let mut fixture = Fixture::new();
    fixture.morph_mesh();
    let input = fixture.times(&[0., 2.]);
    let rotation_view = fixture.view(&[0, 0, 0, 127, 0, 0, 129, 0]);
    let rotation = fixture.accessor(json!({"bufferView":rotation_view,"componentType":5120,"normalized":true,"type":"VEC4","count":2}));
    fixture.channel(1, "rotation", input, rotation, "LINEAR");
    let weight_view = fixture.view(&[128, 127, 0, 0]);
    let weights = fixture.accessor(json!({"bufferView":weight_view,"componentType":5120,"normalized":true,"type":"SCALAR","count":4}));
    fixture.channel(1, "weights", input, weights, "LINEAR");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    let node = &clip.nodes()[0];
    near(
        node.transform()
            .unwrap()
            .sample(Duration::from_secs(1))
            .unwrap()
            .rotation,
        [
            0.,
            0.,
            -std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        ],
    );
    let weights = node.weights().unwrap();
    assert_eq!(weights.weight_count(), 2);
    assert_eq!(weights.sample(Duration::ZERO).unwrap(), [-1., 1.]);
    assert_eq!(weights.sample(Duration::from_secs(1)).unwrap(), [-0.5, 0.5]);
}

#[test]
fn cubic_weights_group_all_components_before_each_derivative_block() {
    let mut fixture = Fixture::new();
    fixture.morph_mesh();
    let input = fixture.times(&[0., 2.]);
    let output = fixture.floats("SCALAR", &[0., 0., 0., 1., 2., -2., 0., 0., 2., 3., 0., 0.]);
    fixture.channel(1, "weights", input, output, "CUBICSPLINE");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    let node = &clip.nodes()[0];
    assert!(node.transform().is_none());
    let weights = node.weights().unwrap();
    assert_eq!(weights.keyframes()[0].out_tangent, [2., -2.]);
    assert_eq!(weights.sample(Duration::from_secs(1)).unwrap(), [1.5, 1.5]);
    let mut output = [0.; 2];
    weights
        .sample_into(Duration::from_secs(2), &mut output)
        .unwrap();
    assert_eq!(output, [2., 3.]);
}

#[test]
fn sparse_and_zero_initialized_animation_accessors_are_supported() {
    let mut fixture = Fixture::new();
    let input = fixture.times(&[0., 2.]);
    let indices = fixture.view(&[1]);
    let data: Vec<_> = [4_f32, 0., 0.]
        .into_iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let values = fixture.view(&data);
    let output = fixture.accessor(json!({"componentType":5126,"type":"VEC3","count":2,"sparse":{"count":1,"indices":{"bufferView":indices,"componentType":5121},"values":{"bufferView":values}}}));
    fixture.channel(1, "translation", input, output, "LINEAR");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    near(
        clip.nodes()[0]
            .transform()
            .unwrap()
            .sample(Duration::from_secs(1))
            .unwrap()
            .translation,
        [2., 0., 0.],
    );
    fixture.json["accessors"][output]
        .as_object_mut()
        .unwrap()
        .remove("sparse");
    let clip = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap();
    near(
        clip.nodes()[0]
            .transform()
            .unwrap()
            .sample(Duration::from_secs(1))
            .unwrap()
            .translation,
        [0.; 3],
    );
}

#[test]
fn malformed_channels_and_numerical_inputs_return_contextual_errors() {
    for times in [
        vec![1., 0.],
        vec![0., 0.],
        vec![0., f32::NAN],
        vec![-1., 0.],
        vec![0., f32::INFINITY],
        vec![0., f32::MAX],
        vec![0., 1e-12],
    ] {
        let mut fixture = Fixture::new();
        let input = fixture.times(&times);
        let output = fixture.floats("VEC3", &[0.; 6]);
        fixture.channel(1, "translation", input, output, "LINEAR");
        let error = fixture
            .prepare()
            .unwrap()
            .animation(0, AnimationOptions::default())
            .unwrap_err();
        assert!(format!("{error:#}").contains("animation 0: channel 0"));
    }
    for mutation in 0..10 {
        let mut fixture = Fixture::new();
        let input = fixture.times(&[0., 2.]);
        let output = fixture.floats("VEC3", &[0.; 6]);
        fixture.channel(1, "translation", input, output, "LINEAR");
        match mutation {
            0 => fixture.json["animations"][0]["channels"][0]["target"]["node"] = json!(99),
            1 => fixture.json["animations"][0]["channels"][0]["target"]["path"] = json!("unknown"),
            2 => fixture.json["animations"][0]["channels"][0]["sampler"] = json!(99),
            3 => fixture.json["animations"][0]["samplers"][0]["output"] = json!(99),
            4 => fixture.json["accessors"][output]["count"] = json!(1),
            5 => fixture.channel(1, "translation", input, output, "LINEAR"),
            6 => {
                fixture.json["nodes"][1]["matrix"] = json!([
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.
                ])
            }
            7 => fixture.json["animations"][0]["channels"][0]["target"]["path"] = json!("scale"),
            8 => {
                fixture.json["animations"][0]["samplers"][0]["interpolation"] = json!("CUBICSPLINE")
            }
            _ => fixture.json["accessors"][output]["type"] = json!("SCALAR"),
        }
        let result = fixture
            .prepare()
            .and_then(|document| document.animation(0, AnimationOptions::default()));
        assert!(result.is_err(), "mutation {mutation}");
    }
    let mut fixture = Fixture::new();
    let input = fixture.times(&[0., 2.]);
    let output = fixture.floats("VEC3", &[0., 0., 0., f32::NAN, 0., 0.]);
    fixture.channel(1, "translation", input, output, "LINEAR");
    assert!(
        fixture
            .prepare()
            .unwrap()
            .animation(0, AnimationOptions::default())
            .is_err()
    );
}

#[test]
fn animation_budgets_charge_repeated_samplers_per_channel_and_allow_retry() {
    let mut fixture = Fixture::new();
    let input = fixture.times(&[0., 2.]);
    let output = fixture.floats("VEC3", &[0.; 6]);
    fixture.channel(1, "translation", input, output, "LINEAR");
    fixture.json["animations"][0]["channels"]
        .as_array_mut()
        .unwrap()
        .push(json!({"sampler":0,"target":{"node":0,"path":"translation"}}));
    let document = fixture.prepare().unwrap();
    for options in [
        AnimationOptions {
            channel_limit: 1,
            ..Default::default()
        },
        AnimationOptions {
            keyframe_limit: 3,
            ..Default::default()
        },
        AnimationOptions {
            scalar_limit: 35,
            ..Default::default()
        },
    ] {
        assert!(document.animation(0, options).is_err());
    }
    let clip = document
        .animation(
            0,
            AnimationOptions {
                channel_limit: 2,
                keyframe_limit: 4,
                scalar_limit: 36,
            },
        )
        .unwrap();
    assert_eq!(clip.nodes().len(), 2);
    assert!(document.animation(1, AnimationOptions::default()).is_err());
}

#[test]
fn incompatible_morph_bindings_and_singular_animation_samples_are_explicit() {
    for mutation in 0..4 {
        let mut fixture = Fixture::new();
        fixture.morph_mesh();
        let input = fixture.times(&[0., 2.]);
        let output = fixture.floats("SCALAR", &[0.; 4]);
        fixture.channel(1, "weights", input, output, "LINEAR");
        match mutation {
            0 => {
                fixture.json["nodes"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("mesh");
            }
            1 => fixture.json["nodes"][1]["weights"] = json!([0.]),
            2 => fixture.json["accessors"][output]["count"] = json!(3),
            _ => {
                let mut primitive = fixture.json["meshes"][0]["primitives"][0].clone();
                primitive["targets"].as_array_mut().unwrap().pop();
                fixture.json["meshes"][0]["primitives"]
                    .as_array_mut()
                    .unwrap()
                    .push(primitive);
            }
        }
        let error = fixture
            .prepare()
            .unwrap()
            .animation(0, AnimationOptions::default())
            .unwrap_err();
        assert!(format!("{error:#}").contains("animation 0: channel 0"));
    }

    let mut fixture = Fixture::new();
    let input = fixture.times(&[0., 2.]);
    let output = fixture.floats("VEC3", &[1., 1., 1., -1., 1., 1.]);
    fixture.channel(1, "scale", input, output, "LINEAR");
    let document = fixture.prepare().unwrap();
    let clip = document.animation(0, AnimationOptions::default()).unwrap();
    let track = clip.nodes()[0].transform().unwrap();
    assert!(track.sample(Duration::from_secs(1)).is_err());
    assert_eq!(
        track.sample(Duration::from_secs(2)).unwrap().scale,
        [-1., 1., 1.]
    );

    fixture.json["animations"][0]["channels"] = json!([]);
    let input = fixture.times(&[0.]);
    let output = fixture.floats("VEC4", &[0., 0., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0.]);
    fixture.channel(1, "rotation", input, output, "CUBICSPLINE");
    let error = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap_err();
    assert!(format!("{error:#}").contains("insufficient keyframes"));

    fixture.json["animations"][0]["channels"] = json!([]);
    let input = fixture.times(&[0., 2.]);
    let output = fixture.floats("VEC4", &[0.; 8]);
    fixture.channel(1, "rotation", input, output, "LINEAR");
    let error = fixture
        .prepare()
        .unwrap()
        .animation(0, AnimationOptions::default())
        .unwrap_err();
    assert!(format!("{error:#}").contains("zero quaternion"));
}
