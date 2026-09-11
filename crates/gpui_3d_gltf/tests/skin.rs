use gpui_3d::{AffineTransform, Camera, Ray, SceneGraph};
use gpui_3d_gltf::{
    AnimationOptions, Document, GeometryOptions, ImageDecodeLimits, Limits, PreparedDocument,
    SceneOptions, SkinOptions,
};
use serde_json::{Value, json};

struct Fixture {
    json: Value,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        let mut fixture = Self {
            json: json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
                "nodes":[{"translation":[5,0,0],"children":[1,3]}, {"children":[2]},
                    {"translation":[0,2,0]}, {"mesh":0,"skin":0,"translation":[100,0,0]}],
                "skins":[{"name":"rig","joints":[2,1],"skeleton":1}],
                "accessors":[],"bufferViews":[],"meshes":[{"primitives":[{"attributes":{}}]}]}),
            bytes: Vec::new(),
        };
        let positions = fixture.floats("VEC3", &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 1.]);
        fixture.json["accessors"][positions]["min"] = json!([0., 0., 0.]);
        fixture.json["accessors"][positions]["max"] = json!([1., 1., 1.]);
        fixture.attr("POSITION", positions);
        let uv = fixture.floats("VEC2", &[0., 0., 1., 0., 1., 1., 0., 1.]);
        fixture.attr("TEXCOORD_0", uv);
        let indices = fixture.raw("SCALAR", 5121, false, 6, &[0, 1, 2, 0, 2, 3]);
        fixture.json["meshes"][0]["primitives"][0]["indices"] = json!(indices);
        let joints = fixture.raw(
            "VEC4",
            5121,
            false,
            4,
            &[1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        );
        fixture.attr("JOINTS_0", joints);
        let weights = fixture.raw(
            "VEC4",
            5121,
            true,
            4,
            &[255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0],
        );
        fixture.attr("WEIGHTS_0", weights);
        let inverse = [
            AffineTransform::from_translation([0., -2., 0.])
                .unwrap()
                .matrix(),
            AffineTransform::IDENTITY.matrix(),
        ];
        let matrices = fixture.floats(
            "MAT4",
            &inverse.into_iter().flatten().flatten().collect::<Vec<_>>(),
        );
        fixture.json["skins"][0]["inverseBindMatrices"] = json!(matrices);
        fixture
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
    fn raw(
        &mut self,
        kind: &str,
        component: u32,
        normalized: bool,
        count: usize,
        bytes: &[u8],
    ) -> usize {
        let view = self.view(bytes);
        let list = self.json["accessors"].as_array_mut().unwrap();
        let index = list.len();
        list.push(json!({"bufferView":view,"componentType":component,"normalized":normalized,"type":kind,"count":count}));
        index
    }
    fn floats(&mut self, kind: &str, values: &[f32]) -> usize {
        let components = match kind {
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            "MAT4" => 16,
            _ => 1,
        };
        self.raw(
            kind,
            5126,
            false,
            values.len() / components,
            &values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
        )
    }
    fn attr(&mut self, semantic: &str, accessor: usize) {
        self.json["meshes"][0]["primitives"][0]["attributes"][semantic] = json!(accessor);
    }
    fn prepare(&self) -> anyhow::Result<PreparedDocument> {
        let mut source = self.json.clone();
        source["buffers"] = json!([{"uri":"skin.bin","byteLength":self.bytes.len()}]);
        Document::from_slice(&serde_json::to_vec(&source)?, Limits::default())?
            .prepare(|_| Ok(self.bytes.clone()))
    }
}

fn near(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }
}

#[test]
fn generated_vertex_splits_preserve_joint_order_and_skinning_inputs() {
    let document = Fixture::new().prepare().unwrap();
    let skin = document.skin(0, SkinOptions::default()).unwrap();
    assert_eq!(skin.joints(), [2, 1]);
    assert_eq!(skin.skeleton(), Some(1));
    assert_eq!(skin.name(), Some("rig"));
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
        assert_eq!(geometry.mesh().vertex_count(), 6);
        let binding = skin.bind(&geometry).unwrap();
        let joints = [
            AffineTransform::from_translation([0., 4., 0.]).unwrap(),
            AffineTransform::IDENTITY,
        ];
        let result = binding.evaluate(geometry.mesh(), &joints).unwrap();
        for (index, (((vertex, base), &source), influences)) in result
            .vertices()
            .iter()
            .zip(geometry.mesh().vertices())
            .zip(geometry.source_vertices())
            .zip(geometry.skin_influences().unwrap())
            .enumerate()
        {
            let expected_joint = if source < 2 { 1 } else { 0 };
            assert_eq!(influences[0].joint, expected_joint);
            let normalized = binding.vertex_influences(index).unwrap();
            assert_eq!(normalized.len(), 1);
            assert_eq!(normalized[0].joint, expected_joint);
            assert_eq!(normalized[0].weight, 1.);
            near(
                vertex.position,
                [
                    base.position[0],
                    base.position[1] + if source < 2 { 0. } else { 2. },
                    base.position[2],
                ],
            );
            near(vertex.normal, base.normal);
        }
        if generate_tangents {
            assert!(result.tangents().is_some());
        }
    }
}

#[test]
fn scene_skins_cancel_mesh_node_transforms_and_keep_instances_and_snapshots_independent() {
    let mut fixture = Fixture::new();
    let times = fixture.floats("SCALAR", &[0., 2.]);
    fixture.json["accessors"][times]["min"] = json!([0.]);
    fixture.json["accessors"][times]["max"] = json!([2.]);
    let values = fixture.floats("VEC3", &[0., 2., 0., 0., 4., 0.]);
    fixture.json["animations"] = json!([{"channels":[{"sampler":0,"target":{"node":2,"path":"translation"}}],
        "samplers":[{"input":times,"output":values}]}]);
    let document = fixture.prepare().unwrap();
    let clip = document.animation(0, AnimationOptions::default()).unwrap();
    let definition =
        std::thread::spawn(move || document.scene(None, SceneOptions::default()).unwrap())
            .join()
            .unwrap();
    let asset = definition
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    assert_eq!(asset.skins().len(), 1);
    assert_eq!(
        asset
            .nodes()
            .iter()
            .find(|node| node.index == 3)
            .unwrap()
            .skin_index,
        Some(0)
    );
    let skin = &asset.skins()[0];
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree()).unwrap();
    let second = graph.instantiate(None, asset.subtree()).unwrap();
    graph
        .set_transform(
            second.root(),
            AffineTransform::from_translation([10., 0., 0.]).unwrap(),
        )
        .unwrap();
    let base = graph.evaluate().unwrap();
    let first_primitive = first.node(skin.primitive()).unwrap();
    let second_primitive = second.node(skin.primitive()).unwrap();
    let rest = base.scene(Camera::default());
    let hit = rest
        .raycast(Ray::new([5.25, 0.75, 5.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(hit.node, Some(first_primitive));
    assert!(
        rest.raycast(Ray::new([105.25, 0.75, 5.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let joint = asset
        .nodes()
        .iter()
        .find(|node| node.index == 2)
        .unwrap()
        .handle;
    for time in [2, 0, 2] {
        let height = 2. + time as f32;
        let locals = [(
            first.node(joint).unwrap(),
            clip.nodes()[0]
                .transform()
                .unwrap()
                .sample_transform(std::time::Duration::from_secs(time))
                .unwrap(),
        )];
        let poses = graph.evaluate_with_transforms(locals).unwrap();
        let first_mesh = skin.evaluate(&first, &poses).unwrap();
        let second_mesh = skin.evaluate(&second, &poses).unwrap();
        graph.set_mesh(first_mesh.0, first_mesh.1).unwrap();
        graph.set_mesh(second_mesh.0, second_mesh.1).unwrap();
        let evaluated = graph.evaluate_with_transforms(locals).unwrap();
        let scene = evaluated.scene(Camera::default());
        let hit = scene
            .raycast(Ray::new([5.25, height - 1.25, 5.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.node, Some(first_primitive));
        let hit = scene
            .raycast(Ray::new([15.25, 0.75, 5.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.node, Some(second_primitive));
        assert!(
            scene
                .raycast(Ray::new([15.25, 2.75, 5.], [0., 0., -1.]).unwrap())
                .is_none()
        );
    }
    assert!(
        rest.raycast(Ray::new([5.25, 2.75, 5.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let foreign = SceneGraph::new();
    assert!(skin.evaluate(&first, &foreign.evaluate().unwrap()).is_err());
    graph.remove_subtree(first.root()).unwrap();
    let remaining = graph.evaluate().unwrap();
    assert!(skin.evaluate(&first, &remaining).is_err());
    assert!(skin.evaluate(&second, &remaining).is_ok());
}

#[test]
fn multiple_joint_sets_normalize_together_and_missing_inverse_binds_use_identity() {
    let mut fixture = Fixture::new();
    fixture.json["skins"][0]
        .as_object_mut()
        .unwrap()
        .remove("inverseBindMatrices");
    for set in [0, 1] {
        let joints: Vec<u8> = (0..4)
            .flat_map(|_| {
                [set as u16, 0, 0, 0]
                    .into_iter()
                    .flat_map(|v| v.to_le_bytes())
            })
            .collect();
        let joints = fixture.raw("VEC4", 5123, false, 4, &joints);
        fixture.attr(&format!("JOINTS_{set}"), joints);
        let weights: Vec<u8> = (0..4)
            .flat_map(|_| {
                [if set == 0 { 16384u16 } else { 49151 }, 0, 0, 0]
                    .into_iter()
                    .flat_map(|v| v.to_le_bytes())
            })
            .collect();
        let weights = fixture.raw("VEC4", 5123, true, 4, &weights);
        fixture.attr(&format!("WEIGHTS_{set}"), weights);
    }
    let document = fixture.prepare().unwrap();
    let skin = document.skin(0, SkinOptions::default()).unwrap();
    let geometry = document.geometry(0, 0, GeometryOptions::default()).unwrap();
    assert_eq!(geometry.influence_count(), 48);
    let binding = skin.bind(&geometry).unwrap();
    let mesh = binding
        .evaluate(
            geometry.mesh(),
            &[
                AffineTransform::from_translation([4., 0., 0.]).unwrap(),
                AffineTransform::IDENTITY,
            ],
        )
        .unwrap();
    for (output, input) in mesh.vertices().iter().zip(geometry.mesh().vertices()) {
        near(
            output.position,
            [
                input.position[0] + 4. * 16384. / 65535.,
                input.position[1],
                input.position[2],
            ],
        );
    }
}

#[test]
fn malformed_joint_attributes_and_skin_references_are_rejected() {
    for mutation in 0..8 {
        let mut fixture = Fixture::new();
        match mutation {
            0 => {
                fixture.json["meshes"][0]["primitives"][0]["attributes"]
                    .as_object_mut()
                    .unwrap()
                    .remove("WEIGHTS_0");
            }
            1 => {
                let attrs = fixture.json["meshes"][0]["primitives"][0]["attributes"]
                    .as_object_mut()
                    .unwrap();
                let joints = attrs.remove("JOINTS_0").unwrap();
                attrs.insert("JOINTS_1".into(), joints);
            }
            2 => {
                let output = fixture.floats("VEC4", &[0.; 16]);
                fixture.attr("WEIGHTS_0", output);
            }
            3 => {
                let output = fixture.floats("VEC4", &[-1.; 16]);
                fixture.attr("WEIGHTS_0", output);
            }
            4 => {
                let output = fixture.floats("VEC4", &[f32::NAN; 16]);
                fixture.attr("WEIGHTS_0", output);
            }
            5 => {
                let output = fixture.raw("VEC4", 5121, false, 4, &[99; 16]);
                fixture.attr("JOINTS_0", output);
            }
            6 => fixture.json["skins"][0]["joints"] = json!([1, 1]),
            _ => fixture.json["skins"][0]["joints"] = json!([1, 99]),
        }
        let result = fixture
            .prepare()
            .and_then(|document| document.scene(None, SceneOptions::default()));
        assert!(result.is_err(), "mutation {mutation}");
    }
    for mutation in 0..3 {
        let mut fixture = Fixture::new();
        match mutation {
            0 => fixture.json["nodes"][1]["children"] = json!([]),
            1 => fixture.json["skins"][0]["skeleton"] = json!(3),
            _ => {
                fixture.json["nodes"][0]["children"] = json!([1, 3]);
                fixture.json["nodes"][1]["children"] = json!([]);
                fixture.json["scenes"][0]["nodes"] = json!([0, 2]);
            }
        }
        assert!(
            fixture
                .prepare()
                .unwrap()
                .scene(None, SceneOptions::default())
                .is_err()
        );
    }
}

#[test]
fn skin_admission_counts_generated_slots_and_shared_scene_bindings() {
    let mut fixture = Fixture::new();
    fixture.json["nodes"][0]["children"] = json!([1, 3, 4]);
    fixture.json["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"mesh":0,"skin":0}));
    let document = fixture.prepare().unwrap();
    assert!(document.skin(0, SkinOptions { joint_limit: 1 }).is_err());
    assert!(
        document
            .geometry(
                0,
                0,
                GeometryOptions {
                    influence_limit: 23,
                    ..Default::default()
                }
            )
            .is_err()
    );
    for options in [
        SceneOptions {
            influence_limit: 47,
            ..Default::default()
        },
        SceneOptions {
            joint_limit: 3,
            ..Default::default()
        },
        SceneOptions {
            deformed_vertex_limit: 11,
            ..Default::default()
        },
    ] {
        assert!(document.scene(None, options).is_err());
    }
    let asset = document
        .scene(
            None,
            SceneOptions {
                influence_limit: 48,
                joint_limit: 4,
                deformed_vertex_limit: 12,
                ..Default::default()
            },
        )
        .unwrap()
        .decode_images(ImageDecodeLimits::default())
        .unwrap();
    assert_eq!(asset.skins().len(), 2);
    assert!(std::ptr::eq(
        asset.skins()[0].joints().as_ptr(),
        asset.skins()[1].joints().as_ptr()
    ));
    assert!(std::ptr::eq(
        asset.skins()[0].binding().inverse_bind_matrices().as_ptr(),
        asset.skins()[1].binding().inverse_bind_matrices().as_ptr()
    ));
    assert!(document.skin(1, SkinOptions::default()).is_err());
}

#[test]
fn inverse_bind_layout_and_unused_vertex_joint_indices_remain_validated() {
    for mutation in 0..4 {
        let mut fixture = Fixture::new();
        let mut matrix = AffineTransform::IDENTITY.matrix();
        match mutation {
            0 => matrix[0][3] = 1.,
            1 => matrix[0][0] = 0.,
            2 => matrix[0][0] = f32::NAN,
            _ => {}
        }
        let values = [matrix, AffineTransform::IDENTITY.matrix()];
        let accessor = fixture.floats(
            "MAT4",
            &values.into_iter().flatten().flatten().collect::<Vec<_>>(),
        );
        if mutation == 3 {
            fixture.json["accessors"][accessor]["count"] = json!(1);
        }
        fixture.json["skins"][0]["inverseBindMatrices"] = json!(accessor);
        let error = fixture
            .prepare()
            .unwrap()
            .skin(0, SkinOptions::default())
            .unwrap_err();
        assert!(format!("{error:#}").contains("skin 0"));
    }

    let mut fixture = Fixture::new();
    let positions = fixture.floats(
        "VEC3",
        &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 1., 0., 0., 0.],
    );
    fixture.json["accessors"][positions]["min"] = json!([0., 0., 0.]);
    fixture.json["accessors"][positions]["max"] = json!([1., 1., 1.]);
    fixture.attr("POSITION", positions);
    let uv = fixture.floats("VEC2", &[0., 0., 1., 0., 1., 1., 0., 1., 0., 0.]);
    fixture.attr("TEXCOORD_0", uv);
    let joints = fixture.raw(
        "VEC4",
        5121,
        false,
        5,
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 99, 0, 0, 0],
    );
    fixture.attr("JOINTS_0", joints);
    let weights = fixture.floats(
        "VEC4",
        &[
            1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.,
        ],
    );
    fixture.attr("WEIGHTS_0", weights);
    let document = fixture.prepare().unwrap();
    let geometry = document.geometry(0, 0, GeometryOptions::default()).unwrap();
    assert!(geometry.source_vertices().iter().all(|&source| source != 4));
    assert!(
        geometry
            .skin_influences()
            .unwrap()
            .flatten()
            .all(|influence| influence.joint == 0)
    );
    let error = document
        .skin(0, SkinOptions::default())
        .unwrap()
        .bind(&geometry)
        .unwrap_err();
    assert!(format!("{error:#}").contains("joint index 99"));
}
