use std::sync::Arc;

use gpui::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, ImageSource, RenderImage,
    TileId, point, rgb, size,
};
use gpui_3d::{
    AffineTransform, Camera, Material, ResolvedTexture, SceneGraph, TextureSource, TextureState,
};
use gpui_3d_gltf::{Document, Limits, PreparedDocument, SceneOptions};
use serde_json::{Value, json};

fn source() -> Value {
    json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
        "nodes":[{"name":"model","children":[1,2],"translation":[0,0,-2]},
            {"name":"part","mesh":0,"translation":[-1,0,0]},
            {"name":"part","mesh":0,"translation":[1,0,0]}],
        "buffers":[{"byteLength":96,"uri":"geometry.bin"}],
        "bufferViews":[{"buffer":0,"byteLength":96,"byteStride":32}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
            {"bufferView":0,"byteOffset":12,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":0,"byteOffset":24,"componentType":5126,"count":3,"type":"VEC2"}],
        "meshes":[{"primitives":[
            {"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"material":0},
            {"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2},"material":1}]}],
        "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[1,0,0,1]}},
            {"pbrMetallicRoughness":{"baseColorFactor":[0,1,0,1]},"alphaMode":"MASK"}]})
}

fn prepared(value: Value) -> PreparedDocument {
    let bytes: Vec<_> = [
        [0_f32, 0., 0., 0., 0., 1., 0., 0.],
        [1., 0., 0., 0., 0., 1., 1., 0.],
        [0., 1., 0., 0., 0., 1., 0., 1.],
    ]
    .into_iter()
    .flatten()
    .flat_map(f32::to_le_bytes)
    .collect();
    Document::from_slice(&serde_json::to_vec(&value).unwrap(), Limits::default())
        .unwrap()
        .prepare(|uri| {
            Ok(if uri == "geometry.bin" {
                bytes.clone()
            } else {
                vec![1, 2, 3]
            })
        })
        .unwrap()
}

fn failure(value: Value, options: SceneOptions) -> String {
    format!("{:#}", prepared(value).scene(None, options).err().unwrap())
}

#[test]
fn scene_instances_preserve_hierarchy_identity_materials_and_shared_geometry() {
    let definition = prepared(source())
        .scene(None, SceneOptions::default())
        .unwrap();
    let asset = definition
        .resolve_images(|_, _| panic!("no texture"))
        .unwrap();
    drop(definition);
    assert_eq!(
        asset.nodes().iter().map(|n| n.index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(asset.nodes()[1].name, asset.nodes()[2].name);
    assert_eq!(
        asset
            .primitives()
            .iter()
            .map(|p| (p.node_index, p.primitive_index, p.material_index))
            .collect::<Vec<_>>(),
        [
            (1, 0, Some(0)),
            (1, 1, Some(1)),
            (2, 0, Some(0)),
            (2, 1, Some(1))
        ]
    );
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree()).unwrap();
    let second = graph.instantiate(None, asset.subtree()).unwrap();
    graph
        .set_transform(
            second.root(),
            AffineTransform::from_translation([0., 2., 0.]).unwrap(),
        )
        .unwrap();
    let first_node = first.node(asset.nodes()[1].handle).unwrap();
    let second_node = second.node(asset.nodes()[1].handle).unwrap();
    assert_eq!(
        graph
            .world_transform(first_node)
            .unwrap()
            .transform_point([0.; 3]),
        [-1., 0., -2.]
    );
    assert_eq!(
        graph
            .world_transform(second_node)
            .unwrap()
            .transform_point([0.; 3]),
        [-1., 2., -2.]
    );
    assert_eq!(
        graph
            .parent(first.node(asset.primitives()[0].handle).unwrap())
            .unwrap(),
        Some(first_node)
    );
    let retained = graph.evaluate().unwrap().scene(Camera::default());
    let frame = retained
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(frame.frame().objects.len(), 8);
    let objects = &frame.frame().objects;
    assert!(Arc::ptr_eq(&objects[0].mesh, &objects[2].mesh));
    assert!(Arc::ptr_eq(&objects[0].mesh, &objects[4].mesh));
    assert_ne!(objects[0].color, objects[1].color);
    for (draw, mapping) in objects.iter().zip(frame.objects()) {
        assert_eq!(frame.object(draw.output_id).unwrap().node, mapping.node);
    }
    for (index, source) in asset.primitives().iter().enumerate() {
        assert_eq!(frame.objects()[index].node, first.node(source.handle));
        assert_eq!(frame.objects()[index + 4].node, second.node(source.handle));
    }
    graph.set_visible(first_node, false).unwrap();
    graph
        .set_material(
            second.node(asset.primitives()[0].handle).unwrap(),
            Material::color(rgb(0x0000ff)),
        )
        .unwrap();
    let changed = graph
        .evaluate()
        .unwrap()
        .scene(Camera::default())
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(changed.frame().objects.len(), 6);
    assert_eq!(changed.frame().objects[2].color, rgb(0x0000ff));
    assert_eq!(frame.frame().objects.len(), 8);
    graph.remove_subtree(first.root()).unwrap();
    assert!(graph.node(second_node).is_ok());
    assert!(graph.node(first_node).is_err());
}

#[test]
fn decoding_is_shared_across_materials_instances_and_retryable() {
    let mut value = source();
    value["images"] = json!([{"uri":"shared.png"}]);
    value["textures"] = json!([{"source":0}]);
    for material in value["materials"].as_array_mut().unwrap() {
        material["pbrMetallicRoughness"]["baseColorTexture"] = json!({"index":0});
    }
    let definition = std::thread::spawn(move || {
        prepared(value)
            .scene(None, SceneOptions::default())
            .unwrap()
    })
    .join()
    .unwrap();
    assert!(
        definition
            .resolve_images(|_, _| anyhow::bail!("unavailable"))
            .is_err()
    );
    let pixels = Arc::new(RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255])),
    )]));
    let mut calls = 0;
    let asset = definition
        .resolve_images(|index, encoded| {
            calls += 1;
            assert_eq!(index, 0);
            assert_eq!(encoded.bytes(), [1, 2, 3]);
            Ok(pixels.clone())
        })
        .unwrap();
    assert_eq!(calls, 1);
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    graph.instantiate(None, asset.subtree()).unwrap();
    let mut requests = 0;
    graph
        .evaluate()
        .unwrap()
        .scene(Camera::default())
        .prepare(1., None, |request| {
            let TextureSource::Image(ImageSource::Render(image)) = request.source else {
                panic!("decoded source expected")
            };
            assert!(Arc::ptr_eq(image, &pixels));
            requests += 1;
            Ok(TextureState::Ready(ResolvedTexture::Image(AtlasTile {
                texture_id: AtlasTextureId {
                    index: 0,
                    kind: AtlasTextureKind::Polychrome,
                },
                tile_id: TileId(0),
                padding: 0,
                bounds: Bounds::new(
                    point(DevicePixels(0), DevicePixels(0)),
                    size(DevicePixels(2), DevicePixels(2)),
                ),
            })))
        })
        .unwrap();
    assert_eq!(requests, 8);
}

#[test]
fn bound_instances_map_occurrences_and_isolate_material_overrides() {
    let mut value = source();
    value["meshes"][0]["primitives"][1]
        .as_object_mut()
        .unwrap()
        .remove("material");
    let asset = prepared(value)
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!("no texture"))
        .unwrap();
    let mut graph = SceneGraph::new();
    let first = asset.instantiate(&mut graph, None).unwrap();
    let second = asset.instantiate(&mut graph, None).unwrap();
    drop(asset);
    assert_eq!(first.asset().nodes().len(), 3);
    assert!(first.node(999).is_none());
    assert!(first.primitive(0, 0).is_none());
    assert!(first.primitive(1, 2).is_none());
    assert!(first.source_node(first.root()).is_none());
    assert!(first.source_primitive(first.root()).is_none());
    for node_index in [1, 2] {
        let group = first.node(node_index).unwrap();
        assert_eq!(first.source_node(group).unwrap().index, node_index);
        assert!(first.source_primitive(group).is_none());
        for primitive_index in [0, 1] {
            let handle = first.primitive(node_index, primitive_index).unwrap();
            let source = first.source_primitive(handle).unwrap();
            assert_eq!(
                (source.node_index, source.primitive_index),
                (node_index, primitive_index)
            );
            assert_eq!(first.subtree_instance().node(source.handle), Some(handle));
            assert_eq!(graph.parent(handle).unwrap(), Some(group));
            assert!(first.source_node(handle).is_none());
            assert!(second.source_primitive(handle).is_none());
        }
        assert!(second.source_node(group).is_none());
    }
    assert_eq!(
        first.material_nodes(Some(0)).collect::<Vec<_>>(),
        [
            first.primitive(1, 0).unwrap(),
            first.primitive(2, 0).unwrap(),
        ]
    );
    assert_eq!(
        first.material_nodes(None).collect::<Vec<_>>(),
        [
            first.primitive(1, 1).unwrap(),
            first.primitive(2, 1).unwrap(),
        ]
    );
    assert_eq!(first.material_nodes(Some(99)).count(), 0);
    let retained = graph.evaluate().unwrap().scene(Camera::default());
    for node in first.material_nodes(Some(0)) {
        graph
            .set_material(node, Material::color(rgb(0x0000ff)))
            .unwrap();
    }
    let changed = graph.evaluate().unwrap().scene(Camera::default());
    let old_frame = retained
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    let frame = changed
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    for (index, object) in frame.frame().objects.iter().enumerate() {
        let handle = frame.objects()[index].node.unwrap();
        let overridden = first
            .source_primitive(handle)
            .is_some_and(|p| p.material_index == Some(0));
        assert_eq!(
            object.color,
            if overridden {
                rgb(0x0000ff)
            } else {
                old_frame.frame().objects[index].color
            }
        );
        assert!(Arc::ptr_eq(
            &object.mesh,
            &old_frame.frame().objects[index].mesh
        ));
    }
    assert_eq!(first.material_nodes(Some(0)).count(), 2);
    let stale = first.primitive(1, 0).unwrap();
    graph.remove_subtree(first.root()).unwrap();
    let replacement = graph.insert(None, gpui_3d::Node::new()).unwrap();
    assert!(graph.node(stale).is_err());
    assert!(first.source_primitive(replacement).is_none());
    assert_eq!(first.source_primitive(stale).unwrap().node_index, 1);
    let live = second.root();
    drop(second);
    assert!(graph.node(live).is_ok());
}

#[test]
fn bound_instantiation_preserves_id_admission_and_graph_identity() {
    let asset = prepared(source())
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!("no texture"))
        .unwrap();
    let mut graph = SceneGraph::new();
    let mut other = SceneGraph::new();
    let foreign = other.insert(None, gpui_3d::Node::new()).unwrap();
    let revision = graph.revision();
    assert!(asset.instantiate(&mut graph, Some(foreign)).is_err());
    assert!(
        asset
            .instantiate_with_ids(&mut graph, None, |_, _| Some("same".into()))
            .is_err()
    );
    assert_eq!(graph.revision(), revision);
    assert!(graph.is_empty());
    let source = asset.nodes()[1].handle;
    let instance = asset
        .instantiate_with_ids(&mut graph, None, |handle, _| {
            (handle == source).then(|| "part".into())
        })
        .unwrap();
    assert_eq!(graph.find(&"part".into()), instance.node(1));
    let other_instance = asset.instantiate(&mut other, None).unwrap();
    assert!(
        instance
            .source_node(other_instance.node(1).unwrap())
            .is_none()
    );
    assert!(
        instance
            .source_primitive(other_instance.primitive(1, 0).unwrap())
            .is_none()
    );
    let revision = graph.revision();
    let count = graph.len();
    assert!(
        asset
            .instantiate_with_ids(&mut graph, None, |handle, _| {
                (handle == source).then(|| "part".into())
            })
            .is_err()
    );
    assert_eq!(graph.len(), count);
    assert_eq!(graph.revision(), revision);
}

#[test]
fn aggregate_limits_count_primitive_occurrences_but_share_mesh_storage() {
    let document = prepared(source());
    let exact = SceneOptions {
        node_limit: 8,
        vertex_limit: 6,
        index_limit: 6,
        ..Default::default()
    };
    assert!(document.scene(None, exact).is_ok());
    for options in [
        SceneOptions {
            node_limit: 7,
            ..exact
        },
        SceneOptions {
            vertex_limit: 5,
            ..exact
        },
        SceneOptions {
            index_limit: 5,
            ..exact
        },
    ] {
        assert!(document.scene(None, options).is_err());
    }
    assert!(document.scene(None, exact).is_ok());
}

#[test]
fn untextured_primitives_preserve_available_uvs_without_requiring_set_zero() {
    let mut value = source();
    for primitive in value["meshes"][0]["primitives"].as_array_mut().unwrap() {
        let attributes = primitive["attributes"].as_object_mut().unwrap();
        let uv = attributes.remove("TEXCOORD_0").unwrap();
        attributes.insert("TEXCOORD_2".into(), uv);
    }
    let asset = prepared(value)
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!("no images"))
        .unwrap();
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    let frame = scene
        .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(frame.frame().objects[0].mesh.vertices()[1].uv, [0., 0.]);
    assert_eq!(frame.frame().objects[0].mesh.uv_at(2, 1), Some([1., 0.]));
}

#[test]
fn independent_material_coordinates_survive_scene_conversion_and_resource_resolution() {
    let mut value = source();
    for primitive in value["meshes"][0]["primitives"].as_array_mut().unwrap() {
        for set in [2, 3, 7, 9] {
            primitive["attributes"][format!("TEXCOORD_{set}")] = json!(2);
        }
    }
    let material = json!({
        "pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":0,"texCoord":2}},
        "emissiveTexture":{"index":0,"texCoord":3},
        "emissiveFactor":[1,1,1],
        "normalTexture":{"index":0,"extensions":{"KHR_texture_transform":{"texCoord":7}}},
        "occlusionTexture":{"index":0,"texCoord":9}
    });
    value["materials"] = json!([material, material]);
    value["textures"] = json!([{"source":0}]);
    value["images"] = json!([{"uri":"map.png","mimeType":"image/png"}]);
    value["extensionsUsed"] = json!(["KHR_texture_transform"]);
    let document = prepared(value);
    let options = SceneOptions {
        tex_coord_limit: 30,
        ..Default::default()
    };
    let definition = document.scene(None, options).unwrap();
    assert_eq!(
        definition
            .geometries()
            .map(|g| g.tex_coord_count())
            .sum::<usize>(),
        30
    );
    for geometry in definition.geometries() {
        assert_eq!(geometry.tex_coord_sets(), [0, 2, 3, 7, 9]);
        assert_eq!(geometry.mesh().tangent_uv_set(), Some(7));
    }
    assert!(
        document
            .scene(
                None,
                SceneOptions {
                    tex_coord_limit: 29,
                    ..options
                }
            )
            .is_err()
    );
    let mut decoded = 0;
    let asset = definition
        .resolve_images(|_, _| {
            decoded += 1;
            Ok(Arc::new(RenderImage::new(vec![image::Frame::new(
                image::RgbaImage::from_pixel(1, 1, image::Rgba([255; 4])),
            )])))
        })
        .unwrap();
    assert_eq!(decoded, 1);
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    let frame = scene
        .prepare(1., None, |_| {
            Ok(TextureState::Ready(ResolvedTexture::Image(AtlasTile {
                texture_id: AtlasTextureId {
                    index: 0,
                    kind: AtlasTextureKind::Polychrome,
                },
                tile_id: TileId(0),
                padding: 0,
                bounds: Bounds::new(
                    point(DevicePixels(0), DevicePixels(0)),
                    size(DevicePixels(1), DevicePixels(1)),
                ),
            })))
        })
        .unwrap();
    assert_eq!(frame.frame().objects.len(), 4);
    for draw in frame.frame().objects.iter() {
        assert_eq!(draw.texture_uv_sets(), [0, 2, 3, 7, 9]);
        assert_eq!(draw.mesh.tangent_uv_set(), Some(7));
    }
}

#[test]
fn selected_scene_preserves_matrix_reflections_and_ignores_unreachable_nodes() {
    let mut value = source();
    value["nodes"][1]
        .as_object_mut()
        .unwrap()
        .remove("translation");
    value["nodes"][1]["matrix"] = json!([-1, 0, 0, 0, 0.25, 1, 0, 0, 0, 0, 1, 0, -1, 0, 0, 1]);
    value["nodes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"scale":[0,0,0]}));
    let document = prepared(value.clone());
    let asset = document
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!())
        .unwrap();
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree()).unwrap();
    let node = instance.node(asset.nodes()[1].handle).unwrap();
    assert_eq!(
        graph
            .world_transform(node)
            .unwrap()
            .transform_point([1., 1., 0.]),
        [-1.75, 1., -2.]
    );
    value["scenes"]
        .as_array_mut()
        .unwrap()
        .push(json!({"nodes":[3]}));
    let document = prepared(value);
    assert!(document.scene(Some(1), SceneOptions::default()).is_err());
    assert!(document.scene(Some(2), SceneOptions::default()).is_err());
}

#[test]
fn invalid_hierarchies_and_unsupported_node_properties_are_contextual_errors() {
    for (nodes, roots) in [
        (json!([{"children":[1]},{"children":[0]}]), json!([0])),
        (json!([{"children":[2]},{"children":[2]},{}]), json!([0, 1])),
        (json!([{}]), json!([0, 0])),
    ] {
        let value =
            json!({"asset":{"version":"2.0"},"nodes":nodes,"scenes":[{"nodes":roots}],"scene":0});
        let error = failure(value, SceneOptions::default());
        assert!(error.contains("node") && error.contains("scene"), "{error}");
    }
    for properties in [
        json!({"scale":[0,1,1]}),
        json!({"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1],"translation":[1,0,0]}),
        json!({"weights":[0.5]}),
    ] {
        let mut value = source();
        value["nodes"][1] = properties;
        assert!(failure(value, SceneOptions::default()).contains("node 1"));
    }
}

#[test]
fn scene_selection_is_explicit_and_deep_hierarchies_are_iterative() {
    let mut value = source();
    value.as_object_mut().unwrap().remove("scene");
    let document = prepared(value);
    assert!(document.scene(None, SceneOptions::default()).is_err());
    assert!(document.scene(Some(0), SceneOptions::default()).is_ok());
    let mut nodes: Vec<Value> = (0..4095).map(|i| json!({"children":[i+1]})).collect();
    nodes.push(json!({"translation":[0,1,0]}));
    let value = json!({"asset":{"version":"2.0"},"nodes":nodes,"scenes":[{"nodes":[0]}],"scene":0});
    let definition = prepared(value)
        .scene(None, SceneOptions::default())
        .unwrap();
    let asset = definition.resolve_images(|_, _| panic!()).unwrap();
    assert_eq!(asset.nodes().len(), 4096);
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree()).unwrap();
    let evaluated = graph.evaluate().unwrap();
    let leaf = instance.node(asset.nodes().last().unwrap().handle).unwrap();
    assert_eq!(
        evaluated.node(leaf).unwrap().world.transform_point([0.; 3]),
        [0., 1., 0.]
    );
}
