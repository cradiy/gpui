use super::*;
use gpui::{MeshPassExpansion3d as Offset, MeshPassSpace3d as Space};

const COLOR: &str = r#"
    fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 1.0, 1.0); }
    fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
"#;

#[test]
fn scene3d_expansion_specializes_vertex_entries_with_and_without_custom_streams() {
    use wgpu::naga;
    for attributes in [
        vec![],
        vec![Scene3dVertexAttribute::new(
            "width",
            wgpu::VertexFormat::Float32,
        )],
    ] {
        let program =
            crate::Scene3dMaterialProgram::compile_with_attributes(COLOR, &attributes).unwrap();
        let module = naga::front::wgsl::parse_str(program.source()).unwrap();
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        for space in [Space::World, Space::Pixels] {
            let mut offset = Offset::new(space, 4.);
            if !attributes.is_empty() {
                offset = offset.weight("width", 1.);
            }
            let constants = Expansion::new(&attributes, Some(&offset))
                .unwrap()
                .constants()
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect();
            let (specialized, _) = naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                Some((naga::ShaderStage::Vertex, "vertex")),
                &constants,
            )
            .unwrap();
            assert_eq!(specialized.entry_points.len(), 1);
            let constant = |name: &str| {
                let (_, value) = specialized
                    .constants
                    .iter()
                    .find(|(_, value)| value.name.as_deref() == Some(name))
                    .unwrap();
                &specialized.global_expressions[value.init]
            };
            assert!(matches!(
                constant("mesh_pass_expansion_mode"),
                naga::Expression::Literal(naga::Literal::U32(value)) if *value == space as u32
            ));
            assert!(matches!(
                constant("mesh_pass_expansion_amount"),
                naga::Expression::Literal(naga::Literal::F32(4.))
            ));
            if !attributes.is_empty() {
                assert!(matches!(
                    constant("mesh_pass_weight_index"),
                    naga::Expression::Literal(naga::Literal::U32(0))
                ));
                assert!(matches!(
                    constant("mesh_pass_weight_limit"),
                    naga::Expression::Literal(naga::Literal::F32(1.))
                ));
            }
        }
        for entry in ["vertex", "shadow_vertex"] {
            naga::back::pipeline_constants::process_overrides(
                &module,
                &info,
                Some((naga::ShaderStage::Vertex, entry)),
                &Default::default(),
            )
            .unwrap();
        }
    }
}

#[test]
fn scene3d_expansion_resolves_scalar_weights_and_keys_all_vertex_controls() {
    let attributes = [
        Scene3dVertexAttribute::new("direction", wgpu::VertexFormat::Float32x3),
        Scene3dVertexAttribute::new("width", wgpu::VertexFormat::Float32),
        Scene3dVertexAttribute::new("tag", wgpu::VertexFormat::Uint32),
    ];
    let world = Offset::new(Space::World, 0.1);
    let weighted = world.clone().weight("width", 2.);
    assert_eq!(
        Expansion::new(&attributes, Some(&weighted)).unwrap().key()[2],
        1
    );
    for name in ["direction", "tag", "missing"] {
        assert!(Expansion::new(&attributes, Some(&world.clone().weight(name, 1.))).is_err());
    }
    let variants = [
        None,
        Some(world.clone()),
        Some(weighted),
        Some(Offset::new(Space::Pixels, 0.1)),
        Some(Offset::new(Space::World, -0.1)),
        Some(world.clone().weight("width", 1.)),
    ];
    let keys: std::collections::HashSet<_> = variants
        .iter()
        .map(|value| Expansion::new(&attributes, value.as_ref()).unwrap().key())
        .collect();
    assert_eq!(keys.len(), variants.len());
    for amount in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(Expansion::new(&attributes, Some(&Offset::new(Space::World, amount))).is_err());
    }
    assert!(Expansion::new(&attributes, Some(&world.weight("width", -1.))).is_err());
    assert!(
        Expansion::new(
            &attributes,
            Some(&Offset::new(Space::World, f32::MAX).weight("width", 2.))
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn scene3d_expansion_moves_color_vertices_but_keeps_primary_data_coverage() -> Result<()> {
    use crate::wgpu_renderer::scene3d::tests::{frame, object};
    use crate::{
        Scene3dChannels, Scene3dMaterialBindingLimits, Scene3dMaterialProgram,
        Scene3dMaterialSource, Scene3dOutputConfig, Scene3dVertexStreamValue, WgpuContext,
        WgpuScene3dRenderer,
    };
    use anyhow::Context as _;
    use std::sync::Arc;
    let context = WgpuContext::new_headless()?;
    let mut object = object();
    object.mesh = gpui::Mesh3d::new(
        object
            .mesh
            .vertices()
            .iter()
            .copied()
            .map(|mut vertex| {
                vertex.normal = [1., 0., 0.];
                vertex
            })
            .collect(),
        object.mesh.indices().to_vec(),
    );
    object.color = gpui::rgb(0xff0000);
    object.unlit = true;
    object.model[3][2] = 0.5;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile_with_attributes(
            COLOR,
            &[Scene3dVertexAttribute::new(
                "width",
                wgpu::VertexFormat::Float32,
            )],
        )?,
    )?;
    let streams = source.bind_vertex_streams(
        3,
        &[(
            "width",
            Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&[2_f32; 3])),
        )],
        12,
    )?;
    let snapshot = source
        .bind([], Scene3dMaterialBindingLimits::default())?
        .with_vertex_streams(streams)?;
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    for offset in [
        Offset::new(Space::World, 0.25),
        Offset::new(Space::Pixels, 8.),
    ] {
        object.mesh_passes = vec![gpui::MeshPass3d {
            material: gpui::MeshMaterial3d::new(Arc::new(snapshot.clone())),
            state: Default::default(),
            expansion: Some(offset.weight("width", 1.)),
        }]
        .into();
        let mut input = frame(&[object.clone()]);
        input.world_to_view[2][2] = -1.;
        let output = renderer.render(
            &input,
            Scene3dOutputConfig {
                size: [64, 64],
                channels: Scene3dChannels::all(),
                color_samples: 4,
            },
        )?;
        let mut readback = output.readback()?;
        context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })?;
        let pixels = readback
            .try_read()?
            .context("expanded color readback not ready")?;
        let expanded = 16 * 64 + 54;
        assert_eq!(
            pixels.linear_rgba.as_ref().unwrap()[expanded],
            [0., 0., 1., 1.]
        );
        assert_eq!(pixels.object_ids.as_ref().unwrap()[expanded], 0);
        assert_eq!(pixels.world_normals.as_ref().unwrap()[expanded], [0.; 4]);
        assert!(
            pixels
                .depth_background
                .is_background(pixels.linear_depth.as_ref().unwrap()[expanded])
        );
        let primary = 24 * 64 + 36;
        assert_eq!(
            pixels.linear_rgba.as_ref().unwrap()[primary],
            [1., 0., 0., 1.]
        );
        assert_eq!(pixels.object_ids.as_ref().unwrap()[primary], 1);
    }
    Ok(())
}
