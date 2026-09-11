use super::*;
use crate::{Scene3dMaterialLimits, Scene3dMaterialProgram};

const MATERIAL: &str = r#"
struct Controls { threshold: vec4<f32> }
@group(1) @binding(0) var<uniform> controls: Controls;
fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    let base = builtin_surface(input, gradients);
    return vec4<f32>(base.rgb, select(0.0, base.a, input.attributes.weight > controls.threshold.x));
}
fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return base * input.attributes.tint + vec3<f32>(f32(input.attributes.region & 1u));
}
"#;

fn attributes() -> Vec<Scene3dVertexAttribute> {
    use wgpu::VertexFormat::*;
    vec![
        Scene3dVertexAttribute::new("weight", Float32),
        Scene3dVertexAttribute::new("tint", Float32x3)
            .interpolation(Scene3dVertexInterpolation::Linear),
        Scene3dVertexAttribute::new("region", Uint32),
    ]
}

#[test]
fn custom_attributes_reach_camera_shadow_and_all_surface_outputs() {
    use wgpu::naga::{self, Binding, BuiltIn, Interpolation, Sampling, TypeInner};
    let attributes = attributes();
    let program = Scene3dMaterialProgram::compile_with_attributes(MATERIAL, &attributes).unwrap();
    assert_eq!(program.vertex_attributes(), attributes);
    assert_eq!(program.resources().len(), 1);
    assert!(program.resources()[0].coverage);
    let module = naga::front::wgsl::parse_str(program.source()).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .unwrap();
    let streams: Vec<_> = module
        .global_variables
        .iter()
        .filter(|(_, g)| g.binding.as_ref().is_some_and(|b| b.group == 2))
        .collect();
    assert_eq!(streams.len(), 3);
    for (index, entry) in module.entry_points.iter().enumerate() {
        if entry.stage == naga::ShaderStage::Vertex {
            assert!(
                entry
                    .function
                    .arguments
                    .iter()
                    .any(|a| a.binding == Some(Binding::BuiltIn(BuiltIn::VertexIndex)))
            );
            for (handle, _) in &streams {
                assert!(
                    info.get_entry_point(index)[*handle].contains(naga::valid::GlobalUse::READ)
                );
            }
            let output = entry.function.result.as_ref().unwrap().ty;
            let TypeInner::Struct { members, .. } = &module.types[output].inner else {
                panic!("vertex output");
            };
            for (location, interpolation, sampling) in [
                (9, Interpolation::Perspective, Sampling::Center),
                (10, Interpolation::Linear, Sampling::Center),
                (11, Interpolation::Flat, Sampling::First),
            ] {
                assert!(members.iter().any(|m| m.binding
                    == Some(Binding::Location {
                        location,
                        interpolation: Some(interpolation),
                        sampling: Some(sampling),
                        blend_src: None,
                        per_primitive: false,
                    })));
            }
        } else {
            // Coverage accesses the uniform through the shared material surface evaluator.
            let control = module
                .global_variables
                .iter()
                .find(|(_, g)| g.name.as_deref() == Some("controls"))
                .unwrap()
                .0;
            assert!(info.get_entry_point(index)[control].contains(naga::valid::GlobalUse::READ));
            for (handle, _) in &streams {
                assert!(info.get_entry_point(index)[*handle].is_empty());
            }
        }
    }
    program.validate_limits(&wgpu::Limits::default()).unwrap();
    for limits in [
        wgpu::Limits {
            max_bind_groups: 2,
            ..Default::default()
        },
        wgpu::Limits {
            max_storage_buffers_per_shader_stage: 2,
            ..Default::default()
        },
        wgpu::Limits {
            max_inter_stage_shader_variables: 11,
            ..Default::default()
        },
    ] {
        assert!(program.validate_limits(&limits).is_err());
    }
}

#[test]
fn custom_attribute_formats_and_interpolation_are_typed() {
    use wgpu::VertexFormat::*;
    for (format, value) in [
        (Float32, "f32(input.attributes.value)"),
        (Float32x2, "input.attributes.value.x"),
        (Float32x3, "input.attributes.value.y"),
        (Float32x4, "input.attributes.value.z"),
        (Sint32, "f32(input.attributes.value)"),
        (Sint32x2, "f32(input.attributes.value.x)"),
        (Sint32x3, "f32(input.attributes.value.y)"),
        (Sint32x4, "f32(input.attributes.value.z)"),
        (Uint32, "f32(input.attributes.value)"),
        (Uint32x2, "f32(input.attributes.value.x)"),
        (Uint32x3, "f32(input.attributes.value.y)"),
        (Uint32x4, "f32(input.attributes.value.z)"),
    ] {
        let material = super::super::DEFAULT.replace(
            "return builtin_surface(input, gradients);",
            &format!("return builtin_surface(input, gradients) * {value};"),
        );
        Scene3dMaterialProgram::compile_with_attributes(
            &material,
            &[Scene3dVertexAttribute::new("value", format)],
        )
        .unwrap();
    }
    let mut declarations = attributes();
    declarations[2].interpolation = Scene3dVertexInterpolation::Perspective;
    assert!(
        Scene3dMaterialProgram::compile_with_attributes(MATERIAL, &declarations)
            .unwrap_err()
            .to_string()
            .contains("integer attribute")
    );
    let mut declarations = attributes();
    declarations[0].format = Unorm8x4;
    assert!(Scene3dMaterialProgram::compile_with_attributes(MATERIAL, &declarations).is_err());
}

#[test]
fn custom_attribute_names_budgets_and_private_storage_are_checked() {
    for name in ["", "1weight", "__weight", "weight;", "fn", "重量"] {
        let mut declarations = attributes();
        declarations[0].name = name.into();
        assert!(Scene3dMaterialProgram::compile_with_attributes(MATERIAL, &declarations).is_err());
    }
    let mut declarations = attributes();
    declarations[1].name = "weight".into();
    assert!(
        Scene3dMaterialProgram::compile_with_attributes(MATERIAL, &declarations)
            .unwrap_err()
            .to_string()
            .contains("duplicate")
    );
    assert!(
        Scene3dMaterialProgram::compile_with_attributes_and_limits(
            MATERIAL,
            &attributes(),
            Scene3dMaterialLimits {
                max_vertex_attributes: 2,
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("count exceeds limit")
    );
    for extra in [
        "fn access() -> u32 { return material_stream_0[0]; }",
        "@group(2) @binding(9) var<storage, read> forged: array<u32>;",
        "fn access(input: SurfaceInput) -> f32 { return dpdx(input.attributes.weight); }",
    ] {
        let material = if extra.contains("dpdx") {
            format!(
                "{}\n{extra}",
                MATERIAL.replace("input.attributes.weight >", "access(input) >")
            )
        } else {
            format!("{MATERIAL}\n{extra}")
        };
        assert!(Scene3dMaterialProgram::compile_with_attributes(&material, &attributes()).is_err());
    }
}
