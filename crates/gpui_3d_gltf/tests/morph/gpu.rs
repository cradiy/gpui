use super::*;
use gpui_3d::{GpuDeformationLimits, Mesh, WgpuContext};
use gpui_3d_gltf::{GpuSceneDeformation, GpuSceneSourceMemory, SceneAsset};

#[path = "rendering.rs"]
mod rendering;

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

#[test]
fn source_budget_counts_primitive_occurrences_and_ignores_static_geometry() -> anyhow::Result<()> {
    let limits = GpuDeformationLimits::default();
    let mut fixture = Fixture::new();
    let single = GpuSceneSourceMemory::plan(&asset(&fixture), limits, None)?;
    assert_eq!(single.primitive_count, 1);
    let primitive = fixture.json["meshes"][0]["primitives"][0].clone();
    fixture.json["meshes"][0]["primitives"] = json!([primitive, primitive]);
    let repeated = asset(&fixture);
    let memory = GpuSceneSourceMemory::plan(&repeated, limits, None)?;
    assert_eq!(memory.primitive_count, 2);
    assert_eq!(memory.source_bytes, single.source_bytes * 2);
    assert_eq!(
        GpuSceneSourceMemory::plan(&repeated, limits, Some(memory.source_bytes))?,
        memory
    );
    assert!(GpuSceneSourceMemory::plan(&repeated, limits, Some(memory.source_bytes - 1)).is_err());
    assert!(
        GpuSceneSourceMemory::plan(
            &repeated,
            GpuDeformationLimits {
                max_source_bytes: 0,
                ..limits
            },
            None
        )
        .is_err()
    );
    assert!(
        GpuSceneSourceMemory::plan(
            &repeated,
            GpuDeformationLimits {
                max_output_bytes: 0,
                ..limits
            },
            None
        )
        .is_err()
    );
    for primitive in fixture.json["meshes"][0]["primitives"]
        .as_array_mut()
        .unwrap()
    {
        primitive.as_object_mut().unwrap().remove("targets");
    }
    assert_eq!(
        GpuSceneSourceMemory::plan(&asset(&fixture), limits, Some(0))?,
        Default::default()
    );
    Ok(())
}

#[test]
fn source_budget_includes_skin_bind_meshes_and_generated_directions() -> anyhow::Result<()> {
    let limits = GpuDeformationLimits::default();
    let plain = asset(&mixed_fixture());
    let memory = GpuSceneSourceMemory::plan(&plain, limits, None)?;
    assert_eq!(memory.primitive_count, 4);
    // Morph sources use six split corners; the Skin-only mesh keeps four vertices.
    // Morph weights and Skin palettes are allocated during evaluation, not upload.
    assert_eq!(
        memory.source_bytes,
        3 * (6 * 64 * 2 + 16 + 6 * 8 + 16)
            + 2 * ((6 + 1 + 2 * 6) * 4 + 32)
            + (4 + 1 + 2 * 4) * 4
            + 32
            + 4 * 64
    );
    let generated = asset(&generated_fixture(false));
    let directions = GpuSceneSourceMemory::plan(&generated, limits, None)?;
    let tangent_bytes = gpui_3d::GpuTangentGenerationMemory::plan(6, limits)?.source_bytes;
    assert_eq!(
        directions.source_bytes - memory.source_bytes,
        3 * (tangent_bytes + 6 * 64)
    );
    assert!(GpuSceneSourceMemory::plan(&generated, limits, Some(memory.source_bytes)).is_err());
    assert_eq!(
        GpuSceneSourceMemory::plan(&generated, limits, Some(directions.source_bytes))?,
        directions
    );
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_zero_weight_tangents_reuse_outputs_with_zero_evaluation_budget() -> anyhow::Result<()> {
    let mut fixture = Fixture::new();
    fixture.json["materials"] = json!([{"normalTexture":{"index":0}}]);
    fixture.json["textures"] = json!([{"source":0}]);
    fixture.json["images"] = json!([{"uri":"normal.png","mimeType":"image/png"}]);
    fixture.json["meshes"][0]["primitives"][0]["material"] = json!(0);
    let asset = asset(&fixture);
    let mut graph = SceneGraph::new();
    let instance = graph.instantiate(None, asset.subtree())?;
    let poses = graph.evaluate()?;
    let gpu = GpuSceneDeformation::new(
        WgpuContext::new_headless()?,
        &asset,
        Default::default(),
        None,
    )?;
    let memory = gpu.evaluation_memory(&instance, &[])?;
    assert_eq!(memory.evaluation_bytes, 0);
    let first = gpu.evaluate(&instance, &poses, &[], Some(0))?;
    let second = gpu.evaluate(&instance, &poses, &[], Some(0))?;
    assert_eq!(memory.output_bytes, first[0].1.buffer().size());
    assert_eq!(first[0].1.buffer().raw(), second[0].1.buffer().raw());
    let weights = [(instance.node(asset.morphs()[0].node()).unwrap(), vec![1.])];
    let changed = gpu.evaluation_memory(&instance, &weights)?;
    assert!(changed.evaluation_bytes > changed.output_bytes);
    assert_eq!(changed.output_bytes, memory.output_bytes);
    let error = gpu
        .evaluate(&instance, &poses, &weights, Some(0))
        .err()
        .unwrap();
    assert!(
        error
            .to_string()
            .contains("exceeding the configured budget")
    );
    Ok(())
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
        let source_memory = GpuSceneSourceMemory::plan(&asset, limits, None)?;
        assert!(
            GpuSceneDeformation::new(
                context.clone(),
                &asset,
                limits,
                Some(source_memory.source_bytes - 1)
            )
            .is_err()
        );
        let gpu = GpuSceneDeformation::new(
            context.clone(),
            &asset,
            limits,
            Some(source_memory.source_bytes),
        )?;
        assert_eq!(gpu.source_memory(), source_memory);
        let target = instance.node(asset.morphs()[0].node()).unwrap();
        for overrides in [
            vec![],
            vec![(target, vec![-0.5])],
            vec![(target, vec![0.])],
            vec![(target, vec![1.])],
            vec![],
        ] {
            let expected = asset.deform(&instance, &poses, &overrides)?;
            let memory = gpu.evaluation_memory(&instance, &overrides)?;
            assert!(
                gpu.evaluate(
                    &instance,
                    &poses,
                    &overrides,
                    Some(memory.evaluation_bytes - 1)
                )
                .is_err()
            );
            let outputs =
                gpu.evaluate(&instance, &poses, &overrides, Some(memory.evaluation_bytes))?;
            assert_eq!(
                memory.output_bytes,
                outputs
                    .iter()
                    .map(|(_, output)| output.buffer().size())
                    .sum::<u64>()
            );
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
        None,
    )?;
    let target = first.node(asset.morphs()[0].node()).unwrap();
    for weights in [
        vec![(target, vec![])],
        vec![(target, vec![f32::NAN])],
        vec![(target, vec![1.]), (target, vec![2.])],
        vec![(first.node(asset.primitives()[0].handle).unwrap(), vec![1.])],
        vec![(second.node(asset.morphs()[0].node()).unwrap(), vec![1.])],
    ] {
        assert!(gpu.evaluation_memory(&first, &weights).is_err());
        assert!(gpu.evaluate(&first, &poses, &weights, None).is_err());
    }
    assert!(
        gpu.evaluate(&first, &SceneGraph::new().evaluate()?, &[], None)
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
            let memory = gpu.evaluation_memory(instance, &overrides)?;
            assert!(
                gpu.evaluate(
                    instance,
                    &poses,
                    &overrides,
                    Some(memory.evaluation_bytes - 1)
                )
                .is_err()
            );
            let outputs =
                gpu.evaluate(instance, &poses, &overrides, Some(memory.evaluation_bytes))?;
            assert_eq!(
                memory.output_bytes,
                outputs
                    .iter()
                    .map(|(_, output)| output.buffer().size())
                    .sum::<u64>()
            );
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
