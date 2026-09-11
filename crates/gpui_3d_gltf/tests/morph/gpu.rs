use super::*;
use gpui_3d::{GpuDeformationLimits, Mesh, WgpuContext};
use gpui_3d_gltf::{GpuSceneDeformation, SceneAsset};

fn asset(fixture: &Fixture) -> SceneAsset {
    fixture
        .prepare()
        .unwrap()
        .scene(None, SceneOptions::default())
        .unwrap()
        .resolve_images(|_, _| {
            Ok(std::sync::Arc::new(gpui::RenderImage::new(vec![
                image::Frame::new(image::RgbaImage::from_pixel(
                    1,
                    1,
                    image::Rgba([255, 128, 128, 255]),
                )),
            ])))
        })
        .unwrap()
}

#[test]
fn asset_admission_preserves_generated_and_authored_tangent_policies() {
    let mut fixture = Fixture::new();
    let flat = asset(&fixture);
    assert!(flat.morphs()[0].geometry().regenerates_normals());
    GpuSceneDeformation::check_asset(&flat).unwrap();

    fixture.json["materials"] = json!([{"normalTexture":{"index":0}}]);
    fixture.json["textures"] = json!([{"source":0}]);
    fixture.json["images"] = json!([{"uri":"normal.png","mimeType":"image/png"}]);
    fixture.json["meshes"][0]["primitives"][0]["material"] = json!(0);
    let generated = asset(&fixture);
    assert_eq!(generated.morphs()[0].default_weights(), [0.]);
    assert!(generated.morphs()[0].geometry().regenerates_tangents());
    GpuSceneDeformation::check_asset(&generated).unwrap();
    assert!(
        generated.morphs()[0]
            .geometry()
            .attribute_targets()
            .base_mesh()
            .tangents()
            .is_none()
    );

    fixture.normals();
    let tangents = fixture.floats("VEC4", &[1., 0., 0., 1.].repeat(4));
    fixture.attribute("TANGENT", tangents);
    let authored = asset(&fixture);
    assert!(!authored.morphs()[0].geometry().regenerates_tangents());
    assert!(
        authored.morphs()[0]
            .geometry()
            .base_mesh()
            .tangents()
            .is_some()
    );
    GpuSceneDeformation::check_asset(&authored).unwrap();
}

fn mixed_fixture() -> Fixture {
    let mut fixture = Fixture::new();
    let joints = fixture.raw("VEC4", 5121, 4, &[0; 16]);
    fixture.attribute("JOINTS_0", joints);
    let influences = fixture.floats("VEC4", &[1., 0., 0., 0.].repeat(4));
    fixture.attribute("WEIGHTS_0", influences);
    let primitive = fixture.json["meshes"][0]["primitives"][0].clone();
    let mut rigid = primitive.clone();
    rigid.as_object_mut().unwrap().remove("targets");
    fixture.json["meshes"] = json!([
        {"primitives":[primitive, primitive],"weights":[0.25]},
        {"primitives":[rigid]},
        {"primitives":[primitive]},
        {"primitives":[rigid]}
    ]);
    let rotation = std::f32::consts::FRAC_1_SQRT_2;
    fixture.json["nodes"] = json!([
        {"children":[1,2,3,4,5]},
        {"rotation":[0.,0.,rotation,rotation],"scale":[1.5,0.75,2.]},
        {"mesh":0,"skin":0,"translation":[8,0,0],"weights":[0.5]},
        {"mesh":1,"skin":0,"translation":[0,3,0]},
        {"mesh":2,"weights":[-0.25]},
        {"mesh":3}
    ]);
    fixture.json["skins"] = json!([{"joints":[1]}]);
    fixture
}

fn generated_fixture(authored_normals: bool) -> Fixture {
    let mut fixture = mixed_fixture();
    let uv = fixture.floats("VEC2", &[0., 0., 0., 1., -1., 1., -1., 0.]);
    let normal = fixture.floats("VEC3", &[0., 0., 1.].repeat(4));
    let normal_delta = fixture.floats("VEC3", &[0.2, -0.1, 0.].repeat(4));
    fixture.json["materials"] = json!([{"normalTexture":{"index":0,"texCoord":2}}]);
    fixture.json["textures"] = json!([{"source":0}]);
    fixture.json["images"] = json!([{"uri":"normal.png","mimeType":"image/png"}]);
    for mesh in fixture.json["meshes"].as_array_mut().unwrap() {
        for primitive in mesh["primitives"].as_array_mut().unwrap() {
            primitive["material"] = json!(0);
            primitive["attributes"]["TEXCOORD_2"] = json!(uv);
            if authored_normals {
                primitive["attributes"]["NORMAL"] = json!(normal);
                if let Some(targets) = primitive["targets"].as_array_mut() {
                    targets[0]["NORMAL"] = json!(normal_delta);
                }
            }
        }
    }
    fixture
}

#[test]
fn generated_tangent_sources_preserve_selected_uvs_corner_order_and_zero_pose() {
    for authored_normals in [false, true] {
        let asset = asset(&generated_fixture(authored_normals));
        GpuSceneDeformation::check_asset(&asset).unwrap();
        for morph in asset.morphs() {
            let geometry = morph.geometry();
            assert!(geometry.regenerates_tangents());
            assert_eq!(geometry.regenerates_normals(), !authored_normals);
            let input = geometry.attribute_targets().base_mesh();
            let generated = input
                .generate_tangents_for_uv_set(2, gpui_3d::TangentGenerationMode::Repair)
                .unwrap();
            assert_eq!(generated.mesh().tangent_uv_set(), Some(2));
            assert_eq!(generated.mesh().indices(), input.indices());
            assert_eq!(
                generated.source_vertices(),
                (0..input.vertex_count() as u32).collect::<Vec<_>>()
            );
            let zero = geometry
                .evaluate(&vec![0.; geometry.targets().len()])
                .unwrap();
            assert!(std::ptr::eq(
                zero.vertices(),
                geometry.base_mesh().vertices()
            ));
            assert_eq!(zero.tangents(), geometry.base_mesh().tangents());
        }
    }
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_imported_tangent_generation_composes_normals_skin_and_retained_zero_samples()
-> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let mut retained = Vec::new();
    for authored_normals in [false, true] {
        let asset = asset(&generated_fixture(authored_normals));
        let mut graph = SceneGraph::new();
        let instance = graph.instantiate(None, asset.subtree())?;
        let poses = graph.evaluate()?;
        let gpu = GpuSceneDeformation::new(context.clone(), &asset, limits)?;
        let target = instance.node(asset.morphs()[0].node()).unwrap();
        for overrides in [
            vec![],
            vec![(target, vec![-0.5])],
            vec![(target, vec![0.])],
            vec![(target, vec![1.])],
            vec![],
        ] {
            let expected = asset.deform(&instance, &poses, &overrides)?;
            let outputs = gpu.evaluate(&instance, &poses, &overrides)?;
            assert_eq!(outputs.len(), expected.len());
            for ((handle, output), (expected_handle, expected)) in outputs.into_iter().zip(expected)
            {
                assert_eq!(handle, expected_handle);
                assert_eq!(output.base_mesh().tangent_uv_set(), Some(2));
                let render_source = output.render_source([2; 5], None)?;
                output.render_geometry(&render_source)?;
                poses.with_meshes([(handle, output.base_mesh().clone())])?;
                retained.push((output, expected));
            }
        }
    }
    for (output, expected) in retained {
        same_mesh(&output.readback()?, &expected);
    }
    Ok(())
}

#[test]
fn node_overrides_cover_all_primitives_and_restore_authored_defaults() {
    let asset = asset(&mixed_fixture());
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree()).unwrap();
    let poses = graph.evaluate().unwrap();
    let target = instance.node(asset.morphs()[0].node()).unwrap();
    let signed = asset
        .deform(&instance, &poses, &[(target, vec![-0.5])])
        .unwrap();
    let defaults = asset.deform(&instance, &poses, &[]).unwrap();
    assert_eq!(signed.len(), 4);
    assert_eq!(asset.primitives().len(), 5);
    for (index, ((handle, mesh), (_, default))) in signed.iter().zip(&defaults).enumerate() {
        assert_eq!(
            *handle,
            instance.node(asset.primitives()[index].handle).unwrap()
        );
        if index < 2 {
            near(mesh.vertices()[5].position, [-8.75, 0., -1.]);
            near(default.vertices()[5].position, [-8.75, 0., 1.]);
        } else {
            same_mesh(mesh, default);
        }
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_imported_deformation_matches_cpu_across_instances_and_retained_samples() -> anyhow::Result<()>
{
    let asset = asset(&mixed_fixture());
    let mut graph = SceneGraph::new();
    let first = graph.instantiate(None, asset.subtree())?;
    let second = graph.instantiate(None, asset.subtree())?;
    graph.set_transform(
        second.root(),
        AffineTransform::from_translation([10., 2., -3.])?,
    )?;
    let poses = graph.evaluate()?;
    let gpu = GpuSceneDeformation::new(
        WgpuContext::new_headless()?,
        &asset,
        GpuDeformationLimits::default(),
    )?;
    let target = first.node(asset.morphs()[0].node()).unwrap();
    for weights in [
        vec![(target, vec![])],
        vec![(target, vec![f32::NAN])],
        vec![(target, vec![1.]), (target, vec![2.])],
        vec![(first.node(asset.primitives()[0].handle).unwrap(), vec![1.])],
        vec![(second.node(asset.morphs()[0].node()).unwrap(), vec![1.])],
    ] {
        assert!(gpu.evaluate(&first, &poses, &weights).is_err());
    }
    assert!(
        gpu.evaluate(&first, &SceneGraph::new().evaluate()?, &[])
            .is_err()
    );
    let mut retained = Vec::new();
    for instance in [&first, &second] {
        let target = instance.node(asset.morphs()[0].node()).unwrap();
        for overrides in [
            vec![],
            vec![(target, vec![-0.5])],
            vec![(target, vec![0.])],
            vec![],
        ] {
            let cpu = asset.deform(instance, &poses, &overrides)?;
            let outputs = gpu.evaluate(instance, &poses, &overrides)?;
            assert_eq!(outputs.len(), cpu.len());
            for (index, ((handle, output), (expected_handle, expected))) in
                outputs.into_iter().zip(cpu).enumerate()
            {
                assert_eq!(handle, expected_handle);
                let primitive = asset.primitives()[index].handle;
                let base = asset
                    .morphs()
                    .iter()
                    .find(|m| m.primitive() == primitive)
                    .map(|m| m.geometry().attribute_targets().base_mesh())
                    .unwrap_or_else(|| {
                        asset
                            .skins()
                            .iter()
                            .find(|s| s.primitive() == primitive)
                            .unwrap()
                            .base_mesh()
                    });
                assert!(std::ptr::eq(output.base_mesh().vertices(), base.vertices()));
                let render_source = output.render_source([0; 5], None)?;
                output.render_geometry(&render_source)?;
                poses.with_meshes([(handle, output.base_mesh().clone())])?;
                retained.push((output, expected));
            }
        }
    }
    drop(gpu);
    drop(asset);
    drop(graph);
    for (output, expected) in retained {
        same_mesh(&output.readback()?, &expected);
    }
    Ok(())
}

fn same_mesh(actual: &Mesh, expected: &Mesh) {
    assert_eq!(actual.indices(), expected.indices());
    assert_eq!(actual.vertex_count(), expected.vertex_count());
    for (actual, expected) in actual.vertices().iter().zip(expected.vertices()) {
        near(actual.position, expected.position);
        near(actual.normal, expected.normal);
        assert_eq!(actual.uv, expected.uv);
    }
    assert_eq!(actual.tangent_uv_set(), expected.tangent_uv_set());
    match (actual.tangents(), expected.tangents()) {
        (Some(actual), Some(expected)) => {
            assert_eq!(actual.len(), expected.len());
            for (actual, expected) in actual.iter().zip(expected) {
                near(
                    [actual[0], actual[1], actual[2]],
                    [expected[0], expected[1], expected[2]],
                );
                assert_eq!(actual[3], expected[3]);
            }
        }
        (None, None) => {}
        _ => panic!("tangent presence mismatch"),
    }
}
