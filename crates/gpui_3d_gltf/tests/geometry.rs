use gpui_3d::{Material, Object, Ray, Scene};
use gpui_3d_gltf::{Document, GeometryOptions, Limits, PreparedDocument};
use serde_json::{Value, json};

struct Fixture {
    json: Value,
    bytes: Vec<u8>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            json: json!({
                "asset":{"version":"2.0"}, "accessors":[], "bufferViews":[],
                "materials":[{}],
                "meshes":[{"primitives":[{"attributes":{},"material":0}]}]
            }),
            bytes: Vec::new(),
        }
    }

    fn view(&mut self, bytes: &[u8], stride: Option<usize>) -> usize {
        self.bytes.resize(self.bytes.len().next_multiple_of(4), 0);
        let offset = self.bytes.len();
        self.bytes.extend(bytes);
        let mut view = json!({"buffer":0,"byteOffset":offset,"byteLength":bytes.len()});
        if let Some(stride) = stride {
            view["byteStride"] = json!(stride);
        }
        let views = self.json["bufferViews"].as_array_mut().unwrap();
        let index = views.len();
        views.push(view);
        index
    }

    fn accessor(&mut self, accessor: Value) -> usize {
        let accessors = self.json["accessors"].as_array_mut().unwrap();
        let index = accessors.len();
        accessors.push(accessor);
        index
    }

    fn attribute<const N: usize>(&mut self, name: &str, data: &[[f32; N]]) -> usize {
        let bytes: Vec<_> = data
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let view = self.view(&bytes, None);
        let mut accessor = json!({"bufferView":view,"componentType":5126,"count":data.len(),"type":format!("VEC{N}")});
        if name == "POSITION" {
            accessor["min"] = json!(
                (0..N)
                    .map(|i| data.iter().map(|v| v[i]).fold(f32::INFINITY, f32::min))
                    .collect::<Vec<_>>()
            );
            accessor["max"] = json!(
                (0..N)
                    .map(|i| data.iter().map(|v| v[i]).fold(f32::NEG_INFINITY, f32::max))
                    .collect::<Vec<_>>()
            );
        }
        let index = self.accessor(accessor);
        self.json["meshes"][0]["primitives"][0]["attributes"][name] = json!(index);
        index
    }

    fn indices(&mut self, values: &[u32], component: u32) {
        let size = match component {
            5121 => 1,
            5123 => 2,
            _ => 4,
        };
        let bytes: Vec<_> = values
            .iter()
            .flat_map(|v| v.to_le_bytes()[..size].to_vec())
            .collect();
        let view = self.view(&bytes, None);
        let index = self.accessor(json!({"bufferView":view,"componentType":component,"count":values.len(),"type":"SCALAR"}));
        self.json["meshes"][0]["primitives"][0]["indices"] = json!(index);
    }

    fn prepare(mut self) -> PreparedDocument {
        if !self.bytes.is_empty() {
            self.json["buffers"] = json!([{"uri":"geometry.bin","byteLength":self.bytes.len()}]);
        }
        Document::from_slice(&serde_json::to_vec(&self.json).unwrap(), Limits::default())
            .unwrap()
            .prepare(|_| Ok(self.bytes.clone()))
            .unwrap()
    }
}

fn triangle() -> Fixture {
    let mut fixture = Fixture::new();
    fixture.attribute("POSITION", &[[0., 0., 0.], [2., 0., 0.], [0., 2., 0.]]);
    fixture
}

fn error(document: &PreparedDocument, options: GeometryOptions) -> String {
    format!("{:#}", document.geometry(0, 0, options).unwrap_err())
}

#[test]
fn vertex_colors_decode_linear_rgb_rgba_and_normalized_components() {
    for component in [5121, 5123, 5126] {
        for channels in [3, 4] {
            let mut fixture = triangle();
            let mut bytes = Vec::new();
            let expected: Vec<[f32; 4]> = (0..3)
                .map(|vertex| {
                    let mut rgba = [1.; 4];
                    for (channel, value) in rgba.iter_mut().enumerate().take(channels) {
                        let n = (vertex * 4 + channel) as u32;
                        *value = match component {
                            5121 => {
                                bytes.push(n as u8);
                                n as f32 / 255.
                            }
                            5123 => {
                                bytes.extend_from_slice(&(n as u16 * 1000).to_le_bytes());
                                (n * 1000) as f32 / 65535.
                            }
                            _ => {
                                let v = n as f32 / 16.;
                                bytes.extend_from_slice(&v.to_le_bytes());
                                v
                            }
                        };
                    }
                    bytes.resize(bytes.len().next_multiple_of(4), 0);
                    rgba
                })
                .collect();
            let stride = (channels
                * if component == 5121 {
                    1
                } else if component == 5123 {
                    2
                } else {
                    4
                })
            .next_multiple_of(4);
            let view = fixture.view(&bytes, Some(stride));
            let accessor = fixture.accessor(json!({"bufferView":view,"componentType":component,"count":3,"type":format!("VEC{channels}"),"normalized":component != 5126}));
            fixture.json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(accessor);
            let geometry = fixture
                .prepare()
                .geometry(0, 0, Default::default())
                .unwrap();
            for (index, &source) in geometry.source_vertices().iter().enumerate() {
                let actual = geometry.mesh().vertex_colors().unwrap()[index];
                for channel in 0..4 {
                    assert!((actual[channel] - expected[source as usize][channel]).abs() < 1e-7);
                }
            }
        }
    }
}

#[test]
fn sparse_vertex_colors_and_invalid_unused_components_are_checked() {
    let mut fixture = triangle();
    let indices = fixture.view(&[1], None);
    let values = fixture.view(&[128, 64, 255, 192], None);
    let color = fixture.accessor(json!({"componentType":5121,"normalized":true,"count":3,"type":"VEC4",
        "sparse":{"count":1,"indices":{"bufferView":indices,"componentType":5121},"values":{"bufferView":values}}}));
    fixture.json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = json!(color);
    let geometry = fixture
        .prepare()
        .geometry(0, 0, Default::default())
        .unwrap();
    let colors = geometry.mesh().vertex_colors().unwrap();
    assert_eq!(colors[0], [0.; 4]);
    assert_eq!(colors[2], [0.; 4]);
    for (actual, expected) in colors[1].iter().zip([128., 64., 255., 192.]) {
        assert!((*actual - expected / 255.).abs() < 1e-7);
    }
    for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        let mut fixture = Fixture::new();
        fixture.attribute(
            "POSITION",
            &[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [2., 2., 0.]],
        );
        fixture.indices(&[0, 1, 2], 5121);
        let mut colors = [[1.; 4]; 4];
        colors[3][2] = value;
        fixture.attribute("COLOR_0", &colors);
        let failure = error(&fixture.prepare(), Default::default());
        assert!(
            failure.contains("COLOR_0") && failure.contains("vertex 3 component 2"),
            "{failure}"
        );
    }
}

#[test]
fn coordinate_admission_bounds_all_sets_and_split_workspace() {
    let mut fixture = triangle();
    fixture.attribute("TEXCOORD_7", &[[0., 0.], [1., 0.], [0., 1.]]);
    fixture.indices(&[0, 1, 2, 0, 1, 2], 5121);
    let document = fixture.prepare();
    let options = GeometryOptions {
        tex_coord_limit: 12,
        ..Default::default()
    };
    let mesh = document.geometry(0, 0, options).unwrap();
    assert_eq!(mesh.tex_coord_sets(), [7]);
    assert_eq!(mesh.mesh().uv_sets().collect::<Vec<_>>(), [0, 7]);
    assert_eq!(mesh.tex_coord_count(), 6);
    assert!(
        error(
            &document,
            GeometryOptions {
                tex_coord_limit: 11,
                ..options
            }
        )
        .contains("texture coordinate limit")
    );
    let mut fixture = triangle();
    fixture.attribute("TEXCOORD_0", &[[0.; 2]; 3]);
    fixture.attribute("TEXCOORD_9", &[[0., 0.], [f32::NAN, 0.], [0., 1.]]);
    assert!(error(&fixture.prepare(), GeometryOptions::default()).contains("TEXCOORD_9"));
}

#[test]
fn authored_tangents_keep_normal_map_coordinate_association() {
    let mut fixture = triangle();
    fixture.attribute("NORMAL", &[[0., 0., 1.]; 3]);
    fixture.attribute("TEXCOORD_7", &[[0., 0.], [0., 1.], [1., 0.]]);
    let tangents = [[0., 1., 0., -1.]; 3];
    fixture.attribute("TANGENT", &tangents);
    let image = fixture.view(&[0], None);
    fixture.json["images"] = json!([{"bufferView":image,"mimeType":"image/png"}]);
    fixture.json["textures"] = json!([{"source":0}]);
    fixture.json["materials"][0]["normalTexture"] = json!({"index":0,"texCoord":7});
    fixture.json["scenes"] = json!([{"nodes":[0]}]);
    fixture.json["nodes"] = json!([{"mesh":0}]);
    let document = fixture.prepare();
    let scene = document.scene(Some(0), Default::default()).unwrap();
    let geometry = scene.geometries().next().unwrap();
    assert_eq!(geometry.tex_coord_sets(), [7]);
    assert_eq!(geometry.mesh().tangent_uv_set(), Some(7));
    assert_eq!(geometry.mesh().tangents().unwrap(), tangents);
    assert_eq!(geometry.mesh().uv_at(0, 1), Some([0., 0.]));
    assert_eq!(geometry.mesh().uv_at(7, 1), Some([0., 1.]));
    document
        .material(Some(0))
        .unwrap()
        .validate_geometry(geometry)
        .unwrap();
}

#[test]
fn indexed_and_nonindexed_meshes_preserve_identity_and_support_world_queries() {
    for component in [None, Some(5121), Some(5123), Some(5125)] {
        let mut fixture = triangle();
        fixture.attribute("NORMAL", &[[0., 0., 2.]; 3]);
        fixture.attribute("TEXCOORD_0", &[[0., 0.], [1., 0.], [0., 1.]]);
        if let Some(component) = component {
            fixture.indices(&[0, 1, 2], component);
        }
        let document = fixture.prepare();
        let converted = document.geometry(0, 0, GeometryOptions::default()).unwrap();
        assert_eq!(
            (
                converted.mesh_index(),
                converted.primitive_index(),
                converted.material_index()
            ),
            (0, 0, Some(0))
        );
        assert_eq!(converted.tex_coord_sets(), [0]);
        assert_eq!(converted.source_vertices(), [0, 1, 2]);
        assert_eq!(converted.mesh().indices(), [0, 1, 2]);
        let scene = Scene::new().object(Object::new(
            converted.mesh().clone(),
            Material::color(gpui::white()),
        ));
        drop(document);
        let hit = scene
            .raycast(Ray::new([0.5, 0.5, 2.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.triangle_index, 0);
        assert_eq!(hit.position, [0.5, 0.5, 0.]);
        assert_eq!(hit.normal, [0., 0., 1.]);
        assert_eq!(hit.uv, [0.25, 0.25]);
    }
}

#[test]
fn strips_and_fans_preserve_winding_and_triangle_order() {
    for (mode, positions, expected) in [
        (
            5,
            [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
            vec![0, 1, 2, 1, 3, 2],
        ),
        (
            6,
            [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]],
            vec![1, 2, 0, 2, 3, 0],
        ),
    ] {
        let mut fixture = Fixture::new();
        fixture.attribute("POSITION", &positions);
        fixture.attribute("NORMAL", &[[0., 0., 1.]; 4]);
        fixture.json["meshes"][0]["primitives"][0]["mode"] = json!(mode);
        let document = fixture.prepare();
        let converted = document.geometry(0, 0, GeometryOptions::default()).unwrap();
        assert_eq!(converted.mesh().indices(), expected);
        for indices in converted.mesh().indices().chunks_exact(3) {
            let [a, b, c] =
                std::array::from_fn(|i| converted.mesh().vertices()[indices[i] as usize].position);
            assert!((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]) > 0.);
        }
        assert!(
            error(
                &document,
                GeometryOptions {
                    index_limit: 5,
                    ..Default::default()
                }
            )
            .contains("index limit")
        );
    }
}

#[test]
fn generated_normals_and_tangents_compose_source_mappings() {
    let mut fixture = Fixture::new();
    let positions = [[0., 0., 0.], [2., 0., 0.], [0., 2., 0.], [0., 0., 1.]];
    let uvs = [[0., 0.], [1., 0.], [0., 1.], [0., 1.]];
    fixture.attribute("POSITION", &positions);
    fixture.attribute("TEXCOORD_0", &uvs);
    // Without normals, authored tangent payloads are not used.
    fixture.attribute("TANGENT", &[[f32::NAN, 0., 0., 0.]; 4]);
    fixture.indices(&[0, 1, 2, 0, 3, 1], 5123);
    let document = fixture.prepare();
    let plain = document.geometry(0, 0, GeometryOptions::default()).unwrap();
    assert_eq!(plain.source_vertices(), [0, 1, 2, 0, 3, 1]);
    assert!(plain.mesh().tangents().is_none());
    let generated = document
        .geometry(
            0,
            0,
            GeometryOptions {
                generate_tangents: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(generated.mesh().triangle_count(), 2);
    assert!(generated.mesh().tangents().is_some());
    for (vertex, &source) in generated
        .mesh()
        .vertices()
        .iter()
        .zip(generated.source_vertices())
    {
        assert_eq!(vertex.position, positions[source as usize]);
        assert_eq!(vertex.uv, uvs[source as usize]);
    }
    assert!(
        error(
            &document,
            GeometryOptions {
                vertex_limit: 4,
                ..Default::default()
            }
        )
        .contains("generated vertex count")
    );
}

#[test]
fn interleaved_attributes_and_normalized_uv_sets_are_decoded_without_flipping() {
    for (component, maximum) in [(5121, 255_u32), (5123, 65535)] {
        let mut fixture = Fixture::new();
        let positions = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]];
        let mut bytes = Vec::new();
        for (i, position) in positions.iter().enumerate() {
            bytes.extend(
                position
                    .iter()
                    .chain([0_f32, 0., 1.].iter())
                    .flat_map(|v| v.to_le_bytes()),
            );
            let uv = [
                if i == 1 { maximum } else { 0 },
                if i == 2 { maximum } else { 0 },
            ];
            for value in uv {
                bytes.extend_from_slice(
                    &value.to_le_bytes()[..if component == 5121 { 1 } else { 2 }],
                );
            }
            bytes.resize((i + 1) * 28, 0);
        }
        let view = fixture.view(&bytes, Some(28));
        let p = fixture.accessor(json!({"bufferView":view,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}));
        let n = fixture.accessor(
            json!({"bufferView":view,"byteOffset":12,"componentType":5126,"count":3,"type":"VEC3"}),
        );
        let uv = fixture.accessor(json!({"bufferView":view,"byteOffset":24,"componentType":component,"normalized":true,"count":3,"type":"VEC2"}));
        fixture.json["meshes"][0]["primitives"][0]["attributes"] =
            json!({"POSITION":p,"NORMAL":n,"TEXCOORD_1":uv});
        fixture.attribute("TEXCOORD_0", &[[0.5, 0.5]; 3]);
        let document = fixture.prepare();
        let converted = document
            .geometry(
                0,
                0,
                GeometryOptions {
                    tangent_uv_set: 1,
                    generate_tangents: true,
                    ..Default::default()
                },
            )
            .unwrap();
        for (index, (vertex, &source)) in converted
            .mesh()
            .vertices()
            .iter()
            .zip(converted.source_vertices())
            .enumerate()
        {
            assert_eq!(vertex.position, positions[source as usize]);
            assert_eq!(vertex.uv, [0.5, 0.5]);
            assert_eq!(
                converted.mesh().uv_at(1, index),
                Some([[0., 0.], [1., 0.], [0., 1.]][source as usize])
            );
        }
        let first = document.geometry(0, 0, GeometryOptions::default()).unwrap();
        assert!(
            first
                .mesh()
                .vertices()
                .iter()
                .all(|vertex| vertex.uv == [0.5, 0.5])
        );
        assert!(
            error(
                &document,
                GeometryOptions {
                    tangent_uv_set: 2,
                    generate_tangents: true,
                    ..Default::default()
                }
            )
            .contains("requires TEXCOORD_2")
        );
    }
}

#[test]
fn degenerate_uv_repairs_retain_triangles_and_scene_diagnostics() {
    let mut fixture = Fixture::new();
    let positions = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]];
    let uv = [[0., 0.], [1., 0.], [1., 1.], [0., 0.]];
    fixture.attribute("POSITION", &positions);
    fixture.attribute("NORMAL", &[[0., 0., 1.]; 4]);
    fixture.attribute("TEXCOORD_0", &uv);
    fixture.indices(&[0, 1, 2, 0, 2, 3], 5121);
    let normal_image = fixture.view(&[0], None);
    fixture.json["images"] = json!([{"bufferView":normal_image,"mimeType":"image/png"}]);
    fixture.json["textures"] = json!([{"source":0}]);
    fixture.json["materials"][0]["normalTexture"] = json!({"index":0});
    fixture.json["scenes"] = json!([{"nodes":[0,1]}]);
    fixture.json["nodes"] = json!([{"mesh":0},{"mesh":0}]);
    let document = fixture.prepare();
    let geometry = document
        .geometry(
            0,
            0,
            GeometryOptions {
                generate_tangents: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(geometry.mesh().triangle_count(), 2);
    assert_eq!(
        geometry.tangent_repairs(),
        &[gpui_3d::TangentRepair {
            triangle: 1,
            corner: 2,
            kind: gpui_3d::TangentRepairKind::OrthonormalBasis
        }]
    );
    for (vertex, &index) in geometry
        .mesh()
        .vertices()
        .iter()
        .zip(geometry.source_vertices())
    {
        assert_eq!(vertex.position, positions[index as usize]);
        assert_eq!(vertex.uv, uv[index as usize]);
    }
    let scene = document.scene(Some(0), Default::default()).unwrap();
    assert_eq!(scene.geometries().len(), 1);
    let scene_geometry = scene.geometries().next().unwrap();
    assert_eq!(scene_geometry.tangent_repairs(), geometry.tangent_repairs());
    assert_eq!(scene_geometry.source_vertices(), geometry.source_vertices());
}

#[test]
fn sparse_positions_and_zero_initialized_accessors_use_bounded_conversion() {
    let mut fixture = Fixture::new();
    let indices = fixture.view(&[1, 2], None);
    let values = fixture.view(
        &[1_f32, 0., 0., 0., 1., 0.]
            .into_iter()
            .flat_map(f32::to_le_bytes)
            .collect::<Vec<_>>(),
        None,
    );
    let p = fixture.accessor(json!({"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0],
        "sparse":{"count":2,"indices":{"bufferView":indices,"componentType":5121},"values":{"bufferView":values}}}));
    let uv = fixture.accessor(json!({"componentType":5126,"count":3,"type":"VEC2"}));
    fixture.json["meshes"][0]["primitives"][0]["attributes"] =
        json!({"POSITION":p,"TEXCOORD_0":uv});
    let document = fixture.prepare();
    let converted = document.geometry(0, 0, GeometryOptions::default()).unwrap();
    assert_eq!(converted.mesh().bounds().max(), [1., 1., 0.]);
    assert!(
        converted
            .mesh()
            .vertices()
            .iter()
            .all(|v| v.normal == [0., 0., 1.] && v.uv == [0.; 2])
    );
    let generated = document
        .geometry(
            0,
            0,
            GeometryOptions {
                generate_tangents: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(generated.mesh().indices(), converted.mesh().indices());
    assert_eq!(generated.tangent_repairs().len(), 3);
    assert!(
        generated
            .tangent_repairs()
            .iter()
            .all(|repair| repair.kind == gpui_3d::TangentRepairKind::OrthonormalBasis)
    );

    let mut huge = Fixture::new();
    let p = huge.accessor(json!({"componentType":5126,"count":4294967295_u64,"type":"VEC3","min":[0,0,0],"max":[0,0,0]}));
    huge.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"] = json!(p);
    assert!(error(&huge.prepare(), GeometryOptions::default()).contains("input vertex count"));
}

#[test]
fn malformed_attributes_and_indices_report_primitive_context() {
    for (component, value) in [(5121, 3), (5121, 255), (5123, 65535), (5125, u32::MAX)] {
        let mut fixture = triangle();
        fixture.indices(&[0, 1, value], component);
        let message = error(&fixture.prepare(), GeometryOptions::default());
        assert!(message.starts_with("mesh 0 primitive 0:"));
        assert!(message.contains(if value == 3 {
            "outside POSITION"
        } else {
            "restart"
        }));
    }
    for normal in [[0., 0., 0.], [f32::NAN, 0., 1.], [f32::INFINITY, 0., 1.]] {
        let mut fixture = triangle();
        fixture.attribute("NORMAL", &[normal; 3]);
        assert!(
            error(&fixture.prepare(), GeometryOptions::default())
                .contains("invalid NORMAL at vertex 0")
        );
    }
    let mut mismatch = triangle();
    mismatch.attribute("NORMAL", &[[0., 0., 1.]; 2]);
    assert!(error(&mismatch.prepare(), GeometryOptions::default()).contains("count differs"));
    for (name, values) in [
        ("NORMAL", vec![[0., 0.]; 3]),
        ("TANGENT", vec![[1., 0.]; 3]),
    ] {
        let mut fixture = triangle();
        fixture.attribute(name, &values);
        assert!(error(&fixture.prepare(), GeometryOptions::default()).contains("format"));
    }
    let mut wrong_uv = triangle();
    wrong_uv.attribute("TEXCOORD_0", &[[0., 0., 0.]; 3]);
    assert!(error(&wrong_uv.prepare(), GeometryOptions::default()).contains("format"));
    let mut colors = triangle();
    colors.attribute("COLOR_1", &[[1., 1., 1.]; 3]);
    assert!(
        error(&colors.prepare(), GeometryOptions::default())
            .contains("unsupported vertex attribute")
    );
    let mut lines = triangle();
    lines.json["meshes"][0]["primitives"][0]["mode"] = json!(3);
    assert!(
        error(&lines.prepare(), GeometryOptions::default()).contains("unsupported primitive mode")
    );
    let plain = triangle().prepare();
    assert!(
        error(
            &plain,
            GeometryOptions {
                generate_tangents: true,
                ..Default::default()
            }
        )
        .contains("requires TEXCOORD_0")
    );
    assert!(plain.geometry(0, 1, GeometryOptions::default()).is_err());
    assert!(plain.geometry(1, 0, GeometryOptions::default()).is_err());
    assert!(plain.geometry(0, 0, GeometryOptions::default()).is_ok());
}

#[test]
fn authored_tangents_are_validated_and_can_be_explicitly_regenerated() {
    let mut fixture = triangle();
    fixture.attribute("NORMAL", &[[0., 0., 1.]; 3]);
    fixture.attribute("TEXCOORD_0", &[[0., 0.], [1., 0.], [0., 1.]]);
    fixture.attribute("TANGENT", &[[1., 0., 0., 0.]; 3]);
    let document = fixture.prepare();
    assert!(error(&document, GeometryOptions::default()).contains("authored tangents"));
    let generated = document
        .geometry(
            0,
            0,
            GeometryOptions {
                generate_tangents: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        generated
            .mesh()
            .tangents()
            .unwrap()
            .iter()
            .all(|t| *t == [1., 0., 0., 1.])
    );
    let mut fixture = triangle();
    fixture.attribute("NORMAL", &[[0., 0., 1.]; 3]);
    fixture.attribute("TANGENT", &[[2., 0., 0., -1.]; 3]);
    let converted = fixture
        .prepare()
        .geometry(0, 0, GeometryOptions::default())
        .unwrap();
    assert_eq!(converted.source_vertices(), [0, 1, 2]);
    assert!(
        converted
            .mesh()
            .tangents()
            .unwrap()
            .iter()
            .all(|t| *t == [1., 0., 0., -1.])
    );
}
