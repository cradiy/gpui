use gpui_3d::{Camera, Ray, SceneGraph};
use gpui_3d_gltf::{Document, GeometryOptions, Limits, PreparedDocument, SceneOptions};
use serde_json::{Value, json};

struct Fixture {
    json: Value,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            json: json!({"asset":{"version":"2.0"},
                "extensionsUsed":["KHR_mesh_quantization"],
                "extensionsRequired":["KHR_mesh_quantization"],
                "accessors":[],"bufferViews":[],
                "meshes":[{"primitives":[{"attributes":{}}]}],
                "scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}]}),
            bytes: Vec::new(),
        }
    }

    fn view(&mut self, bytes: &[u8], stride: Option<usize>) -> usize {
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        let mut view = json!({"buffer":0,"byteOffset":self.bytes.len(),"byteLength":bytes.len()});
        self.bytes.extend_from_slice(bytes);
        if let Some(stride) = stride {
            view["byteStride"] = json!(stride);
        }
        let views = self.json["bufferViews"].as_array_mut().unwrap();
        let index = views.len();
        views.push(view);
        index
    }

    fn accessor(&mut self, value: Value) -> usize {
        let accessors = self.json["accessors"].as_array_mut().unwrap();
        let index = accessors.len();
        accessors.push(value);
        index
    }

    fn vector<const N: usize>(
        &mut self,
        component: u32,
        normalized: bool,
        values: &[[i32; N]],
    ) -> usize {
        let width = if component <= 5121 { 1 } else { 2 };
        let stride = (width * N).next_multiple_of(4);
        let mut bytes = Vec::new();
        for row in values {
            for value in row {
                bytes.extend_from_slice(&value.to_le_bytes()[..width]);
            }
            bytes.resize(bytes.len() + stride - width * N, 0);
        }
        let view = self.view(&bytes, Some(stride));
        self.accessor(json!({"bufferView":view,"componentType":component,
            "normalized":normalized,"count":values.len(),"type":format!("VEC{N}"),
            "min":(0..N).map(|i| values.iter().map(|v| v[i]).min().unwrap()).collect::<Vec<_>>(),
            "max":(0..N).map(|i| values.iter().map(|v| v[i]).max().unwrap()).collect::<Vec<_>>()}))
    }

    fn attr(&mut self, name: &str, index: usize) {
        self.json["meshes"][0]["primitives"][0]["attributes"][name] = json!(index);
    }

    fn triangle(&mut self) -> usize {
        let index = self.vector(5122, false, &[[0, 0, 0], [100, 0, 0], [0, 100, 0]]);
        self.attr("POSITION", index);
        index
    }

    fn prepare(&self) -> anyhow::Result<PreparedDocument> {
        let mut source = self.json.clone();
        source["buffers"] = json!([{"uri":"mesh.bin","byteLength":self.bytes.len()}]);
        Document::from_slice(&serde_json::to_vec(&source)?, Limits::default())?
            .prepare(|_| Ok(self.bytes.clone()))
    }
}

#[test]
fn integer_positions_and_uvs_preserve_signed_normalization_and_raw_ranges() {
    for (component, min, max) in [
        (5120, -128, 127),
        (5121, 0, 255),
        (5122, -32768, 32767),
        (5123, 0, 65535),
    ] {
        for normalized in [false, true] {
            let mut fixture = Fixture::new();
            let position = fixture.vector(
                component,
                normalized,
                &[[min, 0, 0], [max, 0, 0], [0, max, 0]],
            );
            fixture.attr("POSITION", position);
            let uv = fixture.vector(component, normalized, &[[min, max], [max, 0], [0, max]]);
            fixture.attr("TEXCOORD_3", uv);
            let document = fixture.prepare().unwrap();
            let geometry = document.geometry(0, 0, GeometryOptions::default()).unwrap();
            let expected_min = if normalized && min < 0 {
                -1.
            } else {
                min as f32
            };
            let expected_max = if normalized { 1. } else { max as f32 };
            assert_eq!(geometry.source_vertices(), [0, 1, 2]);
            assert_eq!(
                geometry.mesh().vertices()[0].position,
                [expected_min, 0., 0.]
            );
            assert_eq!(
                geometry.mesh().vertices()[1].position,
                [expected_max, 0., 0.]
            );
            assert_eq!(
                geometry.mesh().uv_at(3, 0),
                Some([expected_min, expected_max])
            );
            assert_eq!(geometry.mesh().uv_at(0, 0), Some([0.; 2]));
            assert!(
                document
                    .geometry(
                        0,
                        0,
                        GeometryOptions {
                            vertex_limit: 2,
                            ..Default::default()
                        }
                    )
                    .is_err()
            );
        }
    }
}

#[test]
fn interleaved_signed_normals_and_tangents_are_normalized_without_losing_handedness() {
    let mut fixture = Fixture::new();
    fixture.triangle();
    let view = fixture.view(&[0, 90, 90, 0, 100, 0, 0, 128].repeat(3), Some(8));
    let normal = fixture.accessor(
        json!({"bufferView":view,"componentType":5120,"normalized":true,"count":3,"type":"VEC3"}),
    );
    let tangent = fixture.accessor(json!({"bufferView":view,"byteOffset":4,"componentType":5120,"normalized":true,"count":3,"type":"VEC4"}));
    fixture.attr("NORMAL", normal);
    fixture.attr("TANGENT", tangent);
    let geometry = fixture
        .prepare()
        .unwrap()
        .geometry(0, 0, GeometryOptions::default())
        .unwrap();
    for vertex in geometry.mesh().vertices() {
        assert_eq!(vertex.normal[0], 0.);
        assert!((vertex.normal[1] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((vertex.normal[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
    }
    assert_eq!(geometry.mesh().tangents().unwrap(), [[1., 0., 0., -1.]; 3]);
}

#[test]
fn sparse_quantized_positions_and_morph_deltas_share_decoded_vertex_correspondence() {
    let mut fixture = Fixture::new();
    let indices = fixture.view(&[1, 2], None);
    let values = fixture.view(
        &[100i16, 0, 0, 0, 100, 0]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>(),
        None,
    );
    let position = fixture.accessor(json!({"componentType":5122,"count":3,"type":"VEC3","min":[0,0,0],"max":[100,100,0],
        "sparse":{"count":2,"indices":{"bufferView":indices,"componentType":5121},"values":{"bufferView":values}}}));
    fixture.attr("POSITION", position);
    let delta_indices = fixture.view(&[2], None);
    let delta_values = fixture.view(
        &[0i16, 0, 100]
            .into_iter()
            .flat_map(i16::to_le_bytes)
            .collect::<Vec<_>>(),
        None,
    );
    let delta = fixture.accessor(json!({"componentType":5122,"count":3,"type":"VEC3","min":[0,0,0],"max":[0,0,100],
        "sparse":{"count":1,"indices":{"bufferView":delta_indices,"componentType":5121},"values":{"bufferView":delta_values}}}));
    fixture.json["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION":delta}]);
    fixture.json["meshes"][0]["weights"] = json!([0.5]);
    fixture.json["nodes"][0]["scale"] = json!([0.01, 0.01, 0.01]);
    fixture.json["nodes"][0]["translation"] = json!([2., 3., 4.]);
    let document = fixture.prepare().unwrap();
    let geometry = document.geometry(0, 0, GeometryOptions::default()).unwrap();
    let morph = geometry.morph().unwrap();
    assert_eq!(
        morph.evaluate(&[1.]).unwrap().vertices()[2].position,
        [0., 100., 100.]
    );
    assert_eq!(
        morph.evaluate(&[-1.]).unwrap().vertices()[2].position,
        [0., 100., -100.]
    );
    assert_eq!(geometry.mesh().vertices()[2].position, [0., 100., 0.]);
    let asset = document
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!())
        .unwrap();
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    let hit = scene
        .raycast(Ray::new([2.25, 3.25, 10.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert!((hit.position[2] - 4.125).abs() < 1e-5);
    assert!(hit.node.is_some());
}

#[test]
fn quantized_skin_uses_inverse_bind_dequantization_without_changing_joint_indices() {
    let mut fixture = Fixture::new();
    fixture.triangle();
    let joints = fixture.vector(5121, false, &[[0, 0, 0, 0]; 3]);
    let weights = fixture.vector(5121, true, &[[255, 0, 0, 0]; 3]);
    fixture.attr("JOINTS_0", joints);
    fixture.attr("WEIGHTS_0", weights);
    let matrix: [f32; 16] = [
        0.01, 0., 0., 0., 0., 0.01, 0., 0., 0., 0., 0.01, 0., 0., 0., 0., 1.,
    ];
    let view = fixture.view(
        &matrix
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        None,
    );
    let inverse =
        fixture.accessor(json!({"bufferView":view,"componentType":5126,"count":1,"type":"MAT4"}));
    fixture.json["skins"] = json!([{"joints":[1],"inverseBindMatrices":inverse}]);
    fixture.json["nodes"] = json!([{"mesh":0,"skin":0},{"translation":[0,0,5]}]);
    fixture.json["scenes"][0]["nodes"] = json!([0, 1]);
    let asset = fixture
        .prepare()
        .unwrap()
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!())
        .unwrap();
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    let hit = scene
        .raycast(Ray::new([0.25, 0.25, 10.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert!((hit.position[2] - 5.).abs() < 1e-5);
    assert!(
        scene
            .raycast(Ray::new([25., 25., 10.], [0., 0., -1.]).unwrap())
            .is_none()
    );
}

#[test]
fn normalized_morph_directions_and_zero_initialized_uvs_survive_evaluation() {
    let mut fixture = Fixture::new();
    fixture.triangle();
    let normal = fixture.vector(5122, true, &[[0, 0, 32767]; 3]);
    let tangent = fixture.vector(5122, true, &[[32767, 0, 0, -32768]; 3]);
    fixture.attr("NORMAL", normal);
    fixture.attr("TANGENT", tangent);
    let uv =
        fixture.accessor(json!({"componentType":5120,"normalized":true,"count":3,"type":"VEC2"}));
    fixture.attr("TEXCOORD_1", uv);
    let dp = fixture.vector(5120, true, &[[0, 0, -128]; 3]);
    let dn = fixture.vector(5120, true, &[[127, 0, 0]; 3]);
    let dt = fixture.vector(5122, true, &[[0, 32767, 0]; 3]);
    fixture.json["meshes"][0]["primitives"][0]["targets"] =
        json!([{"POSITION":dp,"NORMAL":dn,"TANGENT":dt}]);
    let geometry = fixture
        .prepare()
        .unwrap()
        .geometry(0, 0, GeometryOptions::default())
        .unwrap();
    let mesh = geometry.morph().unwrap().evaluate(&[1.]).unwrap();
    for (i, vertex) in mesh.vertices().iter().enumerate() {
        assert_eq!(vertex.position[2], -1.);
        assert!((vertex.normal[0] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!((vertex.normal[2] - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        let tangent = mesh.tangents().unwrap()[i];
        assert_eq!(tangent[3], -1.);
        assert!(
            vertex
                .normal
                .iter()
                .zip(tangent)
                .map(|(n, t)| n * t)
                .sum::<f32>()
                .abs()
                < 1e-6
        );
        assert_eq!(mesh.uv_at(1, i), Some([0.; 2]));
    }
}

#[test]
fn undeclared_quantization_invalid_formats_and_alignment_are_rejected() {
    for issue in [
        "undeclared",
        "used_only",
        "required_only",
        "unknown_required",
        "unsigned_normal",
        "raw_normal",
        "misaligned",
        "unsigned_morph",
    ] {
        let mut fixture = Fixture::new();
        let position = fixture.triangle();
        match issue {
            "undeclared" => {
                fixture.json["extensionsUsed"] = json!([]);
                fixture.json["extensionsRequired"] = json!([]);
            }
            "used_only" => fixture.json["extensionsRequired"] = json!([]),
            "required_only" => fixture.json["extensionsUsed"] = json!([]),
            "unknown_required" => fixture.json["extensionsRequired"]
                .as_array_mut()
                .unwrap()
                .push(json!("VENDOR_unknown")),
            "unsigned_normal" | "raw_normal" => {
                let normal = fixture.vector(
                    if issue == "raw_normal" { 5122 } else { 5123 },
                    issue != "raw_normal",
                    &[[0, 0, 100]; 3],
                );
                fixture.attr("NORMAL", normal);
            }
            "misaligned" => {
                let view = fixture.json["accessors"][position]["bufferView"]
                    .as_u64()
                    .unwrap() as usize;
                fixture.json["bufferViews"][view]
                    .as_object_mut()
                    .unwrap()
                    .remove("byteStride");
            }
            "unsigned_morph" => {
                fixture.json["meshes"][0]["primitives"][0]["targets"] = {
                    let delta = fixture.vector(5123, true, &[[0, 0, 0]; 3]);
                    json!([{"POSITION":delta}])
                }
            }
            _ => unreachable!(),
        }
        let result = fixture
            .prepare()
            .and_then(|doc| doc.geometry(0, 0, GeometryOptions::default()));
        let error = format!("{:#}", result.unwrap_err());
        let expected = match issue {
            "used_only" | "required_only" => "both extensionsUsed and extensionsRequired",
            "unknown_required" => "VENDOR_unknown",
            "misaligned" => "four-byte aligned",
            "unsigned_morph" => "unsupported format",
            _ => "unsupported",
        };
        assert!(error.contains(expected), "{issue}: {error}");
    }
}
