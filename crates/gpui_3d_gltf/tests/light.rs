use gpui::{LightKind3d, Scene3dFrame};
use gpui_3d::{Camera, SceneGraph};
use gpui_3d_gltf::{Document, Limits, PreparedDocument, SceneOptions};
use serde_json::{Value, json};

fn source(lights: Value, nodes: Value) -> Value {
    let roots: Vec<_> = (0..nodes.as_array().unwrap().len()).collect();
    json!({"asset":{"version":"2.0"},
        "extensionsUsed":["KHR_lights_punctual"],
        "extensionsRequired":["KHR_lights_punctual"],
        "extensions":{"KHR_lights_punctual":{"lights":lights}},
        "scene":0,"scenes":[{"nodes":roots}],"nodes":nodes})
}

fn prepare(source: &Value) -> PreparedDocument {
    Document::from_slice(&serde_json::to_vec(source).unwrap(), Limits::default())
        .unwrap()
        .prepare(|_| panic!("no external resources"))
        .unwrap()
}

fn frame(graph: &SceneGraph) -> Scene3dFrame {
    graph
        .evaluate()
        .unwrap()
        .scene(Camera::default())
        .prepare(1., None, |_| panic!("no textures"))
        .unwrap()
        .frame()
        .clone()
}

#[test]
fn imported_lights_preserve_linear_colors_parameters_and_emission_conventions() {
    let document = prepare(&source(
        json!([
            {"type":"directional"},
            {"type":"point","color":[0.25,0.003,1.],"intensity":17.,"range":12.},
            {"type":"spot","spot":{"innerConeAngle":0.2,"outerConeAngle":0.7}}
        ]),
        json!([
            {"extensions":{"KHR_lights_punctual":{"light":0}}},
            {"extensions":{"KHR_lights_punctual":{"light":1}}},
            {"extensions":{"KHR_lights_punctual":{"light":2}}}
        ]),
    ));
    assert!(document.light(3).is_err());
    let asset = document
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!("no images"))
        .unwrap();
    assert!(asset.primitives().is_empty());
    assert_eq!(
        asset
            .nodes()
            .iter()
            .map(|n| n.light_index)
            .collect::<Vec<_>>(),
        [Some(0), Some(1), Some(2)]
    );
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    let frame = frame(&graph);
    assert!(frame.objects.is_empty());
    assert!(frame.directional_shadow.is_none());
    let lights = frame.lights.unwrap();
    assert_eq!(lights[0].kind, LightKind3d::Directional);
    assert_eq!(lights[0].direction, [0., 0., 1.]);
    assert_eq!(lights[0].intensity, 1.);
    assert_eq!(lights[0].range, None);
    assert_eq!(lights[1].kind, LightKind3d::Point);
    assert_eq!(lights[1].position, [0.; 3]);
    assert_eq!((lights[1].intensity, lights[1].range), (17., Some(12.)));
    assert!((lights[1].color.r - 0.5370987).abs() < 1e-6);
    assert!((lights[1].color.g - 0.03876).abs() < 1e-6);
    assert!((lights[1].color.b - 1.).abs() < 1e-6);
    assert_eq!(lights[2].kind, LightKind3d::Spot);
    assert_eq!(lights[2].direction, [0., 0., -1.]);
    assert_eq!((lights[2].inner_angle, lights[2].outer_angle), (0.2, 0.7));
}

#[test]
fn shared_light_nodes_follow_hierarchy_and_keep_instance_visibility_independent() {
    let mut source = source(
        json!([
            {"type":"spot","intensity":4.,"range":9.,"spot":{}}
        ]),
        json!([
            {"translation":[3,4,5],"scale":[2,3,-4],"children":[1,2]},
            {"translation":[1,2,3],"camera":0,"extensions":{"KHR_lights_punctual":{"light":0}}},
            {"translation":[0,1,0],"extensions":{"KHR_lights_punctual":{"light":0}}}
        ]),
    );
    source["scenes"][0]["nodes"] = json!([0]);
    source["cameras"] = json!([{"type":"perspective","perspective":{"yfov":1.,"znear":0.1}}]);
    let asset = prepare(&source)
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| panic!())
        .unwrap();
    assert_eq!(asset.nodes()[1].camera_index, Some(0));
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree()).unwrap();
    graph.instantiate(None, asset.subtree()).unwrap();
    let old = graph.evaluate().unwrap();
    let snapshot = frame(&graph);
    let lights = snapshot.lights.unwrap();
    assert_eq!(lights.len(), 4);
    assert_eq!(lights[0].position, [5., 10., -7.]);
    assert_eq!(lights[1].position, [3., 7., 5.]);
    assert_eq!(lights[0].direction, [0., 0., 1.]);
    assert_eq!((lights[0].intensity, lights[0].range), (4., Some(9.)));
    assert_eq!(lights[0].outer_angle, std::f32::consts::FRAC_PI_4);
    graph.set_visible(first.root(), false).unwrap();
    assert_eq!(frame(&graph).lights.unwrap().len(), 2);
    assert_eq!(old.lights().count(), 4);
    let remaining: Vec<_> = graph
        .evaluate()
        .unwrap()
        .lights()
        .map(|(node, _)| node)
        .collect();
    for node in remaining {
        graph.set_visible(node, false).unwrap();
    }
    assert!(frame(&graph).lights.unwrap().is_empty());
}

#[test]
fn light_admission_counts_selected_node_occurrences_not_unique_definitions() {
    let mut source = source(
        json!([{"type":"point"}]),
        json!([
            {"extensions":{"KHR_lights_punctual":{"light":0}}},
            {"extensions":{"KHR_lights_punctual":{"light":0}}}
        ]),
    );
    source["scenes"] = json!([{"nodes":[0,1]},{"nodes":[0]}, {"nodes":[]}]);
    let document = prepare(&source);
    let options = SceneOptions {
        light_limit: 1,
        ..Default::default()
    };
    let error = document.scene(Some(0), options).err().unwrap();
    assert!(format!("{error:#}").contains("node 1: light limit exceeded"));
    document.scene(Some(1), options).unwrap();
    let zero = SceneOptions {
        light_limit: 0,
        ..Default::default()
    };
    assert!(document.scene(Some(1), zero).is_err());
    document.scene(Some(2), zero).unwrap();

    let nodes = vec![json!({"extensions":{"KHR_lights_punctual":{"light":0}}}); 9];
    source["nodes"] = json!(nodes);
    source["scenes"][0]["nodes"] = json!((0..9).collect::<Vec<_>>());
    let document = prepare(&source);
    assert!(document.scene(None, SceneOptions::default()).is_err());
    let asset = document
        .scene(
            None,
            SceneOptions {
                light_limit: 9,
                ..Default::default()
            },
        )
        .unwrap()
        .resolve_images(|_, _| panic!())
        .unwrap();
    let mut graph = SceneGraph::new();
    graph.instantiate(None, asset.subtree()).unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    assert!(scene.prepare(1., None, |_| panic!("no textures")).is_err());
}

#[test]
fn malformed_light_references_and_spot_schema_fail_before_conversion() {
    let valid = source(
        json!([{"type":"spot","spot":{}}]),
        json!([{"extensions":{"KHR_lights_punctual":{"light":0}}}]),
    );
    let mut invalid = Vec::new();
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("extensions");
    invalid.push(missing);
    let mut out_of_range = valid.clone();
    out_of_range["nodes"][0]["extensions"]["KHR_lights_punctual"]["light"] = json!(1);
    invalid.push(out_of_range);
    let mut missing_spot = valid.clone();
    missing_spot["extensions"]["KHR_lights_punctual"]["lights"][0]
        .as_object_mut()
        .unwrap()
        .remove("spot");
    invalid.push(missing_spot);
    let mut unknown_type = valid;
    unknown_type["extensions"]["KHR_lights_punctual"]["lights"][0]["type"] = json!("area");
    invalid.push(unknown_type);
    for source in invalid {
        assert!(
            Document::from_slice(&serde_json::to_vec(&source).unwrap(), Limits::default()).is_err()
        );
    }
}

#[test]
fn invalid_numeric_light_properties_return_contextual_errors() {
    for light in [
        json!({"type":"directional","range":2.}),
        json!({"type":"point","intensity":-1.}),
        json!({"type":"point","intensity":70000.}),
        json!({"type":"point","range":0.}),
        json!({"type":"point","range":1e40}),
        json!({"type":"point","color":[1.1,0.,0.]}),
        json!({"type":"point","color":[-0.1,0.,0.]}),
        json!({"type":"spot","spot":{"innerConeAngle":0.5,"outerConeAngle":0.5}}),
        json!({"type":"spot","spot":{"innerConeAngle":-0.1}}),
        json!({"type":"spot","spot":{"outerConeAngle":2.}}),
    ] {
        let document = prepare(&source(
            json!([light]),
            json!([{"extensions":{"KHR_lights_punctual":{"light":0}}}]),
        ));
        let error = document.scene(None, SceneOptions::default()).err().unwrap();
        assert!(format!("{error:#}").contains("node 0: light 0"));
    }
}
