use std::time::Duration;

use gpui_3d::{AffineTransform, Camera, Ray, SceneGraph};
use gpui_3d_gltf::{
    AnimationOptions, Document, GeometryOptions, ImageDecodeLimits, Limits, PreparedDocument,
    SceneOptions,
};
use serde_json::{Value, json};

struct Fixture {
    json: Value,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let mut result = Self {
            json: json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"mesh":0}],"meshes":[{"primitives":[{"attributes":{}}]}],
                "accessors":[],"bufferViews":[]}),
            bytes: Vec::new(),
        };
        let positions = result.floats("VEC3", &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.]);
        result.extent(positions, [0.; 3], [1., 1., 0.]);
        result.attribute("POSITION", positions);
        let uv = result.floats("VEC2", &[0., 0., 1., 0., 1., 1., 0., 1.]);
        result.attribute("TEXCOORD_0", uv);
        let indices = result.raw("SCALAR", 5121, 6, &[0, 1, 2, 0, 2, 3]);
        result.json["meshes"][0]["primitives"][0]["indices"] = json!(indices);
        let delta = result.floats("VEC3", &[0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 0., 1.]);
        result.extent(delta, [0.; 3], [0., 0., 1.]);
        result.json["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION":delta}]);
        result
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
    fn raw(&mut self, kind: &str, component: u32, count: usize, bytes: &[u8]) -> usize {
        let view = self.view(bytes);
        let accessors = self.json["accessors"].as_array_mut().unwrap();
        let index = accessors.len();
        accessors
            .push(json!({"bufferView":view,"componentType":component,"type":kind,"count":count}));
        index
    }
    fn floats(&mut self, kind: &str, values: &[f32]) -> usize {
        let size = match kind {
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 1,
        };
        self.raw(
            kind,
            5126,
            values.len() / size,
            &values
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
        )
    }
    fn extent(&mut self, index: usize, min: [f32; 3], max: [f32; 3]) {
        self.json["accessors"][index]["min"] = json!(min);
        self.json["accessors"][index]["max"] = json!(max);
    }
    fn attribute(&mut self, name: &str, index: usize) {
        self.json["meshes"][0]["primitives"][0]["attributes"][name] = json!(index);
    }
    fn target(&mut self, name: &str, index: usize) {
        self.json["meshes"][0]["primitives"][0]["targets"][0][name] = json!(index);
    }
    fn source(&self) -> Value {
        let mut json = self.json.clone();
        json["buffers"] = json!([{"uri":"mesh.bin","byteLength":self.bytes.len()}]);
        json
    }
    fn prepare(&self) -> anyhow::Result<PreparedDocument> {
        Document::from_slice(&serde_json::to_vec(&self.source())?, Limits::default())?
            .prepare(|_| Ok(self.bytes.clone()))
    }
    fn normals(&mut self) {
        let normals = self.floats("VEC3", &[0., 0., 1.].repeat(4));
        self.attribute("NORMAL", normals);
    }
    fn translation_target(&mut self) {
        let delta = self.floats("VEC3", &[1., 0., 0.].repeat(4));
        self.extent(delta, [1., 0., 0.], [1., 0., 0.]);
        self.target("POSITION", delta);
    }
}

fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
}

#[test]
fn bent_faces_regenerate_directions_without_changing_corner_correspondence() {
    let document = Fixture::new().prepare().unwrap();
    for generate_tangents in [false, true] {
        let geometry = document
            .geometry(
                0,
                0,
                GeometryOptions {
                    generate_tangents,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(geometry.source_vertices(), [0, 1, 2, 0, 2, 3]);
        assert_eq!(geometry.mesh().vertex_count(), 6);
        let morph = geometry.morph().unwrap();
        for weight in [1., -1., 0., 1.] {
            let mesh = morph.evaluate(&[weight]).unwrap();
            assert_eq!(mesh.indices(), geometry.mesh().indices());
            assert!(std::ptr::eq(
                mesh.indices().as_ptr(),
                geometry.mesh().indices().as_ptr()
            ));
            for (i, vertex) in mesh.vertices().iter().enumerate() {
                let source = geometry.source_vertices()[i];
                let base = geometry.mesh().vertices()[i];
                near(
                    vertex.position,
                    [
                        base.position[0],
                        base.position[1],
                        if source == 3 { weight } else { 0. },
                    ],
                );
                let scale = (2. * weight * weight + 1.).sqrt();
                near(
                    vertex.normal,
                    if i < 3 {
                        [0., 0., 1.]
                    } else {
                        [weight / scale, -weight / scale, 1. / scale]
                    },
                );
                if let Some(tangents) = mesh.tangents() {
                    let tangent = tangents[i];
                    let dot = vertex
                        .normal
                        .iter()
                        .zip(tangent)
                        .map(|(a, b)| a * b)
                        .sum::<f32>();
                    assert!(dot.abs() < 1e-5);
                    let expected = if i < 3 {
                        [1., 0., 0.]
                    } else {
                        let len = (1. + weight * weight).sqrt();
                        [1. / len, 0., -weight / len]
                    };
                    near([tangent[0], tangent[1], tangent[2]], expected);
                }
            }
        }
    }
}

#[test]
fn authored_direction_deltas_keep_signed_weights_and_tangent_handedness() {
    let mut fixture = Fixture::new();
    fixture.normals();
    let tangents = fixture.floats("VEC4", &[1., 0., 0., -1.].repeat(4));
    fixture.attribute("TANGENT", tangents);
    let normal_delta = fixture.floats("VEC3", &[0., 1., 0.].repeat(4));
    fixture.target("NORMAL", normal_delta);
    let tangent_delta = fixture.floats("VEC3", &[0., 1., 0.].repeat(4));
    fixture.target("TANGENT", tangent_delta);
    let document = fixture.prepare().unwrap();
    let geometry = document.geometry(0, 0, GeometryOptions::default()).unwrap();
    let morph = geometry.morph().unwrap();
    assert_eq!(geometry.mesh().vertex_count(), 4);
    let base = morph.evaluate(&[0.]).unwrap();
    assert!(std::ptr::eq(
        base.vertices().as_ptr(),
        geometry.mesh().vertices().as_ptr()
    ));
    let mesh = morph.evaluate(&[-1.]).unwrap();
    near(mesh.vertices()[3].position, [0., 1., -1.]);
    near(
        mesh.vertices()[0].normal,
        [0., -1. / 2f32.sqrt(), 1. / 2f32.sqrt()],
    );
    let tangent = mesh.tangents().unwrap()[0];
    let length = 1.5f32.sqrt();
    near(
        [tangent[0], tangent[1], tangent[2]],
        [1. / length, -0.5 / length, -0.5 / length],
    );
    assert_eq!(tangent[3], -1.);
    assert!(morph.evaluate(&[]).is_err());
    assert!(morph.evaluate(&[f32::NAN]).is_err());
    assert!(morph.evaluate(&[f32::INFINITY]).is_err());
    assert!(morph.evaluate(&[1.]).is_ok());
}

#[test]
fn sparse_and_zero_initialized_targets_preserve_unmodified_vertices() {
    let mut fixture = Fixture::new();
    let delta = fixture.json["meshes"][0]["primitives"][0]["targets"][0]["POSITION"]
        .as_u64()
        .unwrap() as usize;
    fixture.json["accessors"][delta]
        .as_object_mut()
        .unwrap()
        .remove("bufferView");
    let indices = fixture.view(&[3]);
    let values = fixture.view(
        &[0f32, 0., 2.]
            .into_iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    );
    fixture.json["accessors"][delta]["sparse"] = json!({"count":1,"indices":{"bufferView":indices,"componentType":5121},"values":{"bufferView":values}});
    fixture.extent(delta, [0.; 3], [0., 0., 2.]);
    let geometry = fixture
        .prepare()
        .unwrap()
        .geometry(0, 0, GeometryOptions::default())
        .unwrap();
    let result = geometry.morph().unwrap().evaluate(&[0.5]).unwrap();
    near(result.vertices()[5].position, [0., 1., 1.]);
    near(result.vertices()[0].position, [0.; 3]);
    fixture.json["accessors"][delta]
        .as_object_mut()
        .unwrap()
        .remove("sparse");
    let geometry = fixture
        .prepare()
        .unwrap()
        .geometry(0, 0, GeometryOptions::default())
        .unwrap();
    let result = geometry.morph().unwrap().evaluate(&[8.]).unwrap();
    for (actual, base) in result.vertices().iter().zip(geometry.mesh().vertices()) {
        near(actual.position, base.position);
        near(actual.normal, base.normal);
        assert_eq!(actual.uv, base.uv);
    }
}

#[test]
fn missing_normals_ignore_authored_tangent_displacements_but_validate_their_data() {
    let mut fixture = Fixture::new();
    let tangent = fixture.floats("VEC4", &[1., 0., 0., 1.].repeat(4));
    fixture.attribute("TANGENT", tangent);
    let delta = fixture.floats("VEC3", &[0., 2., 0.].repeat(4));
    fixture.json["meshes"][0]["primitives"][0]["targets"] = json!([{"TANGENT":delta}]);
    let document = fixture.prepare().unwrap();
    for generate_tangents in [false, true] {
        let geometry = document
            .geometry(
                0,
                0,
                GeometryOptions {
                    generate_tangents,
                    ..Default::default()
                },
            )
            .unwrap();
        let output = geometry.morph().unwrap().evaluate(&[2.]).unwrap();
        for (vertex, base) in output.vertices().iter().zip(geometry.mesh().vertices()) {
            near(vertex.position, base.position);
            near(vertex.normal, [0., 0., 1.]);
        }
        if generate_tangents {
            assert!(
                output
                    .tangents()
                    .unwrap()
                    .iter()
                    .all(|value| *value == [1., 0., 0., 1.])
            );
        } else {
            assert!(output.tangents().is_none());
        }
    }
    let malformed = fixture.floats("VEC3", &[0., f32::NAN, 0.].repeat(4));
    fixture.target("TANGENT", malformed);
    let document = fixture.prepare().unwrap();
    let error = document
        .geometry(0, 0, GeometryOptions::default())
        .unwrap_err();
    assert!(format!("{error:#}").contains("nonfinite morph deltas"));
}

#[test]
fn regenerated_tangents_use_changed_positions_with_authored_normal_deltas() {
    let mut fixture = Fixture::new();
    fixture.normals();
    let delta = fixture.floats("VEC3", &[0., 1., 0.].repeat(4));
    fixture.target("NORMAL", delta);
    let geometry = fixture
        .prepare()
        .unwrap()
        .geometry(
            0,
            0,
            GeometryOptions {
                generate_tangents: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(geometry.source_vertices(), [0, 1, 2, 0, 2, 3]);
    let mesh = geometry.morph().unwrap().evaluate(&[0.5]).unwrap();
    assert_eq!(mesh.indices(), geometry.mesh().indices());
    for (vertex, tangent) in mesh.vertices().iter().zip(mesh.tangents().unwrap()) {
        near(
            vertex.normal,
            [0., 0.5 / 1.25f32.sqrt(), 1. / 1.25f32.sqrt()],
        );
        let dot = vertex
            .normal
            .iter()
            .zip(tangent)
            .map(|(a, b)| a * b)
            .sum::<f32>();
        assert!(dot.abs() < 1e-5);
    }
    assert_ne!(
        mesh.tangents().unwrap()[5],
        geometry.mesh().tangents().unwrap()[5]
    );
}

#[test]
fn weight_animation_deforms_before_skinning_and_keeps_instances_independent() {
    let mut fixture = Fixture::new();
    fixture.normals();
    fixture.translation_target();
    fixture.json["nodes"] = json!([{"children":[1,2]}, {"rotation":[0.,0.,std::f32::consts::FRAC_1_SQRT_2,std::f32::consts::FRAC_1_SQRT_2]}, {"mesh":0,"skin":0,"translation":[100,0,0],"weights":[0.5]}]);
    fixture.json["skins"] = json!([{"joints":[1]}]);
    fixture.json["meshes"][0]["weights"] = json!([0.25]);
    let joints = fixture.raw("VEC4", 5121, 4, &[0; 16]);
    fixture.attribute("JOINTS_0", joints);
    let influences = fixture.floats("VEC4", &[1., 0., 0., 0.].repeat(4));
    fixture.attribute("WEIGHTS_0", influences);
    let times = fixture.floats("SCALAR", &[0., 2.]);
    fixture.json["accessors"][times]["min"] = json!([0.]);
    fixture.json["accessors"][times]["max"] = json!([2.]);
    let weights = fixture.floats("SCALAR", &[0., 2.]);
    fixture.json["animations"] = json!([{"channels":[{"sampler":0,"target":{"node":2,"path":"weights"}}],"samplers":[{"input":times,"output":weights}]}]);
    let document = fixture.prepare().unwrap();
    let clip = document.animation(0, AnimationOptions::default()).unwrap();
    let asset = document
        .scene(None, SceneOptions::default())
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree()).unwrap();
    let second = graph.instantiate(None, asset.subtree()).unwrap();
    graph
        .set_transform(
            second.root(),
            AffineTransform::from_translation([10., 0., 0.]).unwrap(),
        )
        .unwrap();
    let morph = &asset.morphs()[0];
    assert_eq!(morph.default_weights(), [0.5]);
    let target = first.node(morph.node()).unwrap();
    let retained = graph.evaluate().unwrap().scene(Camera::default());
    assert!(
        retained
            .raycast(Ray::new([-0.5, 0.75, 5.], [0., 0., -1.]).unwrap())
            .is_some()
    );
    for time in [2, 0, 2] {
        let values = clip.nodes()[0]
            .weights()
            .unwrap()
            .sample(Duration::from_secs(time))
            .unwrap();
        let poses = graph.evaluate().unwrap();
        let meshes = asset.deform(&first, &poses, &[(target, values)]).unwrap();
        for (node, mesh) in meshes {
            graph.set_mesh(node, mesh).unwrap();
        }
        let scene = graph.evaluate().unwrap().scene(Camera::default());
        let hit = scene
            .raycast(Ray::new([-0.5, time as f32 + 0.5, 5.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.node, first.node(morph.primitive()));
        let hit = scene
            .raycast(Ray::new([9.5, 0.75, 5.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.node, second.node(morph.primitive()));
    }
    assert!(
        retained
            .raycast(Ray::new([-0.5, 2.5, 5.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let poses = graph.evaluate().unwrap();
    for overrides in [
        vec![(target, vec![])],
        vec![(target, vec![f32::NAN])],
        vec![(target, vec![1.]), (target, vec![2.])],
        vec![(second.node(morph.node()).unwrap(), vec![1.])],
    ] {
        assert!(asset.deform(&first, &poses, &overrides).is_err());
        assert_eq!(graph.evaluate().unwrap().revision(), poses.revision());
    }
    let replacements = asset.deform(&first, &poses, &[]).unwrap();
    near(replacements[0].1.vertices()[0].position, [-100., 0.5, 0.]);
    let foreign = SceneGraph::new();
    assert!(
        asset
            .deform(&first, &foreign.evaluate().unwrap(), &[])
            .is_err()
    );
    graph.remove_subtree(first.root()).unwrap();
    assert!(
        asset
            .deform(&first, &graph.evaluate().unwrap(), &[])
            .is_err()
    );
    assert!(
        asset
            .deform(&second, &graph.evaluate().unwrap(), &[])
            .is_ok()
    );
}

#[test]
fn default_weights_and_scene_admission_follow_primitive_occurrences() {
    let mut fixture = Fixture::new();
    fixture.translation_target();
    fixture.json["nodes"] = json!([{"mesh":0},{"mesh":0,"weights":[-1.]}]);
    fixture.json["scenes"][0]["nodes"] = json!([0, 1]);
    fixture.json["meshes"][0]["weights"] = json!([0.25]);
    let document = fixture.prepare().unwrap();
    let options = SceneOptions {
        vertex_limit: 6,
        index_limit: 6,
        morph_target_limit: 1,
        morph_attribute_limit: 6,
        deformed_vertex_limit: 12,
        ..Default::default()
    };
    let asset = document
        .scene(None, options)
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    assert_eq!(asset.morphs()[0].default_weights(), [0.25]);
    assert_eq!(asset.morphs()[1].default_weights(), [-1.]);
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree()).unwrap();
    let meshes = asset
        .deform(&instance, &graph.evaluate().unwrap(), &[])
        .unwrap();
    near(meshes[0].1.vertices()[0].position, [0.25, 0., 0.]);
    near(meshes[1].1.vertices()[0].position, [-1., 0., 0.]);
    for limited in [
        SceneOptions {
            deformed_vertex_limit: 11,
            ..options
        },
        SceneOptions {
            morph_target_limit: 0,
            ..options
        },
        SceneOptions {
            morph_attribute_limit: 5,
            ..options
        },
    ] {
        assert!(document.scene(None, limited).is_err());
    }
    assert!(document.scene(None, options).is_ok());
    let attributes = asset.morphs()[0].geometry().targets()[0]
        .positions
        .as_ref()
        .unwrap();
    assert!(std::ptr::eq(
        attributes.as_ptr(),
        asset.morphs()[1].geometry().targets()[0]
            .positions
            .as_ref()
            .unwrap()
            .as_ptr()
    ));
}

#[test]
fn invalid_target_metadata_and_data_are_rejected_without_consuming_resources() {
    for kind in [
        "count",
        "base",
        "bounds",
        "bounds_order",
        "nonfinite",
        "target_count",
        "weights",
    ] {
        let mut fixture = Fixture::new();
        let delta = fixture.json["meshes"][0]["primitives"][0]["targets"][0]["POSITION"]
            .as_u64()
            .unwrap() as usize;
        match kind {
            "count" => fixture.json["accessors"][delta]["count"] = json!(3),
            "base" => fixture.target("NORMAL", delta),
            "bounds" => {
                fixture.json["accessors"][delta]
                    .as_object_mut()
                    .unwrap()
                    .remove("min");
            }
            "bounds_order" => fixture.extent(delta, [1.; 3], [0.; 3]),
            "nonfinite" => {
                let value = fixture.floats("VEC3", &[f32::INFINITY, 0., 0.].repeat(4));
                fixture.extent(value, [0.; 3], [1.; 3]);
                fixture.target("POSITION", value);
            }
            "target_count" => {
                let mut other = fixture.json["meshes"][0]["primitives"][0].clone();
                other.as_object_mut().unwrap().remove("targets");
                fixture.json["meshes"][0]["primitives"]
                    .as_array_mut()
                    .unwrap()
                    .push(other);
            }
            "weights" => fixture.json["meshes"][0]["weights"] = json!([0., 1.]),
            _ => unreachable!(),
        }
        let document = fixture.prepare().unwrap();
        let error = document
            .geometry(0, 0, GeometryOptions::default())
            .unwrap_err();
        let expected = match kind {
            "count" => "float VEC3",
            "base" => "no base attribute",
            "bounds" => "requires min and max",
            "bounds_order" => "invalid POSITION morph bounds",
            "nonfinite" => "nonfinite morph deltas",
            "target_count" => "different morph target counts",
            "weights" => "match target count",
            _ => unreachable!(),
        };
        assert!(format!("{error:#}").contains(expected), "{kind}: {error:#}");
        assert!(document.geometry(0, 0, GeometryOptions::default()).is_err());
    }
}

#[test]
fn unsupported_target_semantics_are_not_silently_discarded_in_json_or_glb() {
    let mut fixture = Fixture::new();
    let other = fixture.json["meshes"][0]["primitives"][0].clone();
    fixture.json["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
        .push(other);
    let uv = fixture.json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        .as_u64()
        .unwrap() as usize;
    fixture.target("TEXCOORD_0", uv);
    let mut json = serde_json::to_vec(&fixture.source()).unwrap();
    json.resize(json.len().next_multiple_of(4), b' ');
    let mut glb = Vec::new();
    glb.extend(b"glTF");
    glb.extend(2u32.to_le_bytes());
    glb.extend(((20 + json.len()) as u32).to_le_bytes());
    glb.extend((json.len() as u32).to_le_bytes());
    glb.extend(b"JSON");
    glb.extend(&json);
    for bytes in [&json, &glb] {
        let document = Document::from_slice(bytes, Limits::default())
            .unwrap()
            .prepare(|_| Ok(fixture.bytes.clone()))
            .unwrap();
        let error = document
            .geometry(0, 0, GeometryOptions::default())
            .unwrap_err();
        assert!(format!("{error:#}").contains("unsupported attribute TEXCOORD_0"));
        assert!(document.geometry(0, 1, GeometryOptions::default()).is_ok());
    }
}

#[test]
fn unused_source_deltas_still_consume_input_admission_and_require_finite_values() {
    let mut fixture = Fixture::new();
    let indices = fixture.json["meshes"][0]["primitives"][0]["indices"]
        .as_u64()
        .unwrap() as usize;
    fixture.json["accessors"][indices]["count"] = json!(3);
    let document = fixture.prepare().unwrap();
    let options = GeometryOptions {
        morph_attribute_limit: 3,
        ..Default::default()
    };
    let error = document.geometry(0, 0, options).unwrap_err();
    assert!(format!("{error:#}").contains("morph input attributes exceed limit"));
    let geometry = document
        .geometry(
            0,
            0,
            GeometryOptions {
                morph_attribute_limit: 4,
                ..options
            },
        )
        .unwrap();
    assert_eq!(geometry.source_vertices(), [0, 1, 2]);
    let delta = fixture.floats(
        "VEC3",
        &[0., 0., 0., 0., 0., 0., 0., 0., 0., f32::NAN, 0., 0.],
    );
    fixture.extent(delta, [0.; 3], [0.; 3]);
    fixture.target("POSITION", delta);
    let error = fixture
        .prepare()
        .unwrap()
        .geometry(0, 0, GeometryOptions::default())
        .unwrap_err();
    assert!(format!("{error:#}").contains("nonfinite morph deltas"));
}
