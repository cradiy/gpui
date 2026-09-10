use super::*;
use futures::executor::block_on;
use gpui_3d_gltf::SceneLoadStatus;
use serde_json::json;
use std::path::Path;

fn animated_fixture(root: &Path, singular: bool) -> PathBuf {
    let path = fixture(root);
    let mut source: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let mut bytes = std::fs::read(root.join("mesh.bin")).unwrap();
    let mut accessor = |kind: &str, values: &[f32], components: usize| {
        let offset = bytes.len();
        bytes.extend(values.iter().flat_map(|v| v.to_le_bytes()));
        let view = source["bufferViews"].as_array().unwrap().len();
        source["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":values.len()*4}));
        let index = source["accessors"].as_array().unwrap().len();
        source["accessors"].as_array_mut().unwrap().push(json!({"bufferView":view,"componentType":5126,"count":values.len()/components,"type":kind}));
        index
    };
    let times = accessor("SCALAR", &[2., 4.], 1);
    let positions = accessor("VEC3", &[0., 0., 0., 4., 0., 0.], 3);
    let weights = accessor("SCALAR", &[0., 1.], 1);
    let influences = accessor("VEC4", &[1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.], 4);
    let scales = accessor("VEC3", &[1., 1., 1., -1., 1., 1.], 3);
    let view = source["bufferViews"].as_array().unwrap().len();
    source["bufferViews"]
        .as_array_mut()
        .unwrap()
        .push(json!({"buffer":0,"byteOffset":bytes.len(),"byteLength":12}));
    bytes.extend([0; 12]);
    let joints = source["accessors"].as_array().unwrap().len();
    source["accessors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":view,"componentType":5121,"count":3,"type":"VEC4"}));
    source["accessors"][times]["min"] = json!([2.]);
    source["accessors"][times]["max"] = json!([4.]);
    source["buffers"][0]["byteLength"] = json!(bytes.len());
    source["nodes"][0]["children"] = json!([1, 2]);
    source["nodes"][1]["skin"] = json!(0);
    source["nodes"]
        .as_array_mut()
        .unwrap()
        .extend([json!({"name":"Joint"}), json!({"name":"Other scene"})]);
    source["skins"] = json!([{"joints":[2]}]);
    source["meshes"][0]["weights"] = json!([0.]);
    source["meshes"][0]["primitives"][0]["targets"] = json!([{"POSITION":0}]);
    source["meshes"][0]["primitives"][0]["attributes"]["JOINTS_0"] = json!(joints);
    source["meshes"][0]["primitives"][0]["attributes"]["WEIGHTS_0"] = json!(influences);
    source["animations"] = json!([{"name":"Deform", "samplers":[
        {"input":times,"output":positions}, {"input":times,"output":weights}, {"input":times,"output":scales}],
        "channels":[{"sampler":0,"target":{"node":2,"path":"translation"}},
            {"sampler":1,"target":{"node":1,"path":"weights"}},
            {"sampler":0,"target":{"node":3,"path":"translation"}}]}]);
    if singular {
        source["animations"][0]["channels"]
            .as_array_mut()
            .unwrap()
            .push(json!({"sampler":2,"target":{"node":1,"path":"scale"}}));
    }
    std::fs::write(root.join("mesh.bin"), bytes).unwrap();
    std::fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    path
}

#[test]
fn playback_random_access_combines_joint_motion_and_morph_before_queries() {
    let temporary = tempfile::tempdir().unwrap();
    let path = animated_fixture(temporary.path(), false);
    let mut slot = SceneLoadSlot::new();
    let mut model = None;
    let completion = block_on(
        slot.begin()
            .run(load(&path, None, Some(0), &ImageCache::default())),
    );
    assert!(
        publish(&mut slot, &mut model, completion),
        "{:?}",
        slot.error()
    );
    let mut model = model.unwrap();
    assert_eq!(
        model.playback.as_ref().unwrap().time(),
        Duration::from_secs(2)
    );
    assert_eq!(model.skipped_tracks, 1);
    let mut first = None;
    for offset in [1, 2, 0, 1] {
        model
            .control(|p| {
                p.seek(Duration::from_secs(offset));
                Ok(())
            })
            .unwrap();
        let bounds = model.evaluated.bounds().unwrap();
        let shift = offset as f32 * 2.;
        assert_eq!(bounds.min(), [3. + shift, 2., 0.]);
        assert_eq!(
            bounds.max(),
            [5. + shift + offset as f32, 4. + offset as f32, 0.]
        );
        if offset == 1 {
            if let Some(first) = first {
                assert_eq!(bounds, first);
            } else {
                first = Some(bounds);
            }
            let viewport = Bounds::new(gpui::point(px(0.), px(0.)), size(px(640.), px(360.)));
            let camera = Camera::orbit(0., 0., 5.)
                .frame_bounds(bounds, 640. / 360., 1.2)
                .unwrap();
            let point = camera
                .world_to_screen(viewport, [5.5, 2.5, 0.])
                .unwrap()
                .unwrap();
            let hit = model
                .evaluated
                .scene(camera)
                .pick(viewport, point.position)
                .unwrap();
            assert_eq!(
                model
                    .instance
                    .source_primitive(hit.node.unwrap())
                    .unwrap()
                    .node_index,
                1
            );
        }
    }
}

#[test]
fn invalid_sample_retains_the_last_frame_and_playback_position() {
    let temporary = tempfile::tempdir().unwrap();
    let path = animated_fixture(temporary.path(), true);
    let mut slot = SceneLoadSlot::new();
    let mut model = None;
    let completion = block_on(
        slot.begin()
            .run(load(&path, None, Some(0), &ImageCache::default())),
    );
    assert!(
        publish(&mut slot, &mut model, completion),
        "{:?}",
        slot.error()
    );
    let mut model = model.unwrap();
    let original = model.evaluated.bounds();
    assert!(
        model
            .control(|p| {
                p.seek(Duration::from_secs(1));
                Ok(())
            })
            .is_err()
    );
    assert_eq!(model.evaluated.bounds(), original);
    assert_eq!(model.playback.as_ref().unwrap().position(), Duration::ZERO);
    model
        .control(|p| {
            p.seek(Duration::from_secs(2));
            Ok(())
        })
        .unwrap();
    assert_ne!(model.evaluated.bounds(), original);
}

fn fixture(root: &Path) -> PathBuf {
    let vertices: Vec<u8> = [
        0_f32, 0., 0., 2., 0., 0., 0., 2., 0., 0., 0., 1., 0., 0., 1.,
    ]
    .into_iter()
    .flat_map(f32::to_le_bytes)
    .collect();
    std::fs::write(root.join("mesh.bin"), vertices).unwrap();
    image::RgbaImage::from_pixel(1, 1, image::Rgba([90, 120, 180, 255]))
        .save(root.join("color map.png"))
        .unwrap();
    let source = json!({
        "asset":{"version":"2.0"}, "scene":0,
        "buffers":[{"uri":"mesh.bin","byteLength":60}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36}, {"buffer":0,"byteOffset":36,"byteLength":24}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[2,2,0]},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
        "images":[{"uri":"color%20map.png"}], "textures":[{"source":0}],
        "materials":[{"name":"Coating","doubleSided":true,"pbrMetallicRoughness":{"metallicFactor":0.2,"baseColorTexture":{"index":0}}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0}]}],
        "nodes":[{"translation":[3,2,0],"children":[1]},{"name":"Panel","mesh":0}],
        "scenes":[{"nodes":[0]}]
    });
    let path = root.join("scene.gltf");
    std::fs::write(&path, serde_json::to_vec(&source).unwrap()).unwrap();
    path
}

#[test]
fn background_file_loading_publishes_bound_geometry_and_material_metadata() {
    let temporary = tempfile::tempdir().unwrap();
    let path = fixture(temporary.path());
    let mut slot = SceneLoadSlot::new();
    let request = slot.begin();
    let completion = std::thread::spawn(move || {
        let queue = SceneLoadQueue::new(1.try_into().unwrap(), 1);
        block_on(
            request
                .run(queue.run(|| async { load(&path, None, None, &ImageCache::default()).await })),
        )
    })
    .join()
    .unwrap();
    let mut model = None;
    assert!(publish(&mut slot, &mut model, completion));
    let model = model.unwrap();
    let bounds = model.evaluated.bounds().unwrap();
    assert_eq!(bounds.min(), [3., 2., 0.]);
    assert_eq!(bounds.max(), [5., 4., 0.]);
    let viewport = Bounds::new(gpui::point(px(23.), px(17.)), size(px(640.), px(360.)));
    let camera = Camera::orbit(0., 0., 5.)
        .frame_bounds(bounds, 640. / 360., 1.2)
        .unwrap();
    let point = camera
        .world_to_screen(viewport, [3.5, 2.5, 0.])
        .unwrap()
        .unwrap();
    let hit = model
        .evaluated
        .scene(camera)
        .pick(viewport, point.position)
        .unwrap();
    let primitive = model.instance.source_primitive(hit.node.unwrap()).unwrap();
    assert_eq!(
        (
            primitive.node_index,
            primitive.mesh_index,
            primitive.material_index
        ),
        (1, 0, Some(0))
    );
    let details = model.details(hit.node).join("\n");
    assert!(
        details.contains("Panel")
            && details.contains("Coating")
            && details.contains("Metallic 0.20")
    );
}

#[test]
fn failed_and_superseded_reloads_preserve_the_displayed_model() {
    let temporary = tempfile::tempdir().unwrap();
    let path = fixture(temporary.path());
    let images = ImageCache::default();
    let mut slot = SceneLoadSlot::new();
    let mut model = None;
    let ready = block_on(slot.begin().run(load(&path, None, None, &images)));
    assert!(publish(&mut slot, &mut model, ready));
    let root = model.as_ref().unwrap().instance.root();
    let old = block_on(slot.begin().run(load(&path, None, None, &images)));
    let current = slot.begin();
    assert!(!publish(&mut slot, &mut model, old));
    assert_eq!(model.as_ref().unwrap().instance.root(), root);
    std::fs::write(&path, b"invalid json").unwrap();
    let failed = block_on(current.run(load(&path, None, None, &images)));
    assert!(!publish(&mut slot, &mut model, failed));
    assert_eq!(slot.status(), SceneLoadStatus::Failed);
    assert!(slot.error().is_some());
    assert_eq!(model.as_ref().unwrap().instance.root(), root);
    assert_eq!(slot.asset().unwrap().primitives().len(), 1);
}

#[test]
fn fixed_camera_aspect_fits_portrait_and_landscape_stages_without_stretching() {
    for available in [size(px(900.), px(300.)), size(px(300.), px(900.))] {
        for aspect in [0.5, 2.] {
            let fitted = fitted_size(available, Some(aspect));
            assert!(fitted.width <= available.width && fitted.height <= available.height);
            assert_eq!(fitted.width / fitted.height, aspect);
            assert!(fitted.width == available.width || fitted.height == available.height);
        }
        assert_eq!(fitted_size(available, None), available);
    }
}
