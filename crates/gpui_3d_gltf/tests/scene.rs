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
    assert_eq!(frame.frame().objects[0].mesh.vertices()[1].uv, [1., 0.]);
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
