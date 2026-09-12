use super::*;

const SOURCE: &str = r#"
struct Controls { tint: vec4<f32>, basis: mat3x3<f32>, gain: f32 }
struct Spare { value: vec4<f32> }
@group(1) @binding(7) var surface_map: texture_2d<f32>;
@group(1) @binding(3) var<uniform> controls: Controls;
@group(1) @binding(12) var environment_map: texture_cube<f32>;
@group(1) @binding(8) var map_sampler: sampler;
@group(1) @binding(17) var<uniform> spare: Spare;
fn sample_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return textureSampleGrad(surface_map, map_sampler, input.uv.xy, gradients[0], gradients[1]);
}
fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return sample_surface(input, gradients) * controls.tint;
}
fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return base + textureSampleLevel(environment_map, map_sampler, unit_vector(input.normal), 0.0).rgb * controls.gain;
}
"#;

#[test]
fn material_resources_reflect_layout_and_transitive_evaluator_usage() {
    let program = MaterialProgram::compile(SOURCE).unwrap();
    let resources = program.resources();
    assert_eq!(
        resources
            .iter()
            .map(|r| (r.binding, r.name.as_str(), r.coverage, r.shading))
            .collect::<Vec<_>>(),
        [
            (3, "controls", true, true),
            (7, "surface_map", true, false),
            (8, "map_sampler", true, true),
            (12, "environment_map", false, true),
            (17, "spare", false, false),
        ]
    );
    assert_eq!(
        resources[0].kind,
        Scene3dMaterialResourceKind::Uniform { min_size: 80 }
    );
    assert_eq!(
        resources[4].kind,
        Scene3dMaterialResourceKind::Uniform { min_size: 16 }
    );
    assert_eq!(
        resources[1].layout_entry().ty,
        wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        }
    );
    assert_eq!(
        resources[3].kind,
        Scene3dMaterialResourceKind::Texture {
            dimension: wgpu::TextureViewDimension::Cube
        }
    );
    for resource in resources {
        let entry = resource.layout_entry();
        assert_eq!(entry.binding, resource.binding);
        assert_eq!(entry.visibility, wgpu::ShaderStages::FRAGMENT);
        assert_eq!(entry.count, None);
    }
    let retained = program.clone();
    drop(program);
    assert_eq!(retained.resources().len(), 5);
}

#[test]
fn material_resource_admission_includes_unused_declarations_and_device_reservations() {
    let limits = Scene3dMaterialLimits {
        max_source_bytes: SOURCE.len(),
        max_resources: 5,
        max_vertex_attributes: 0,
        max_uniform_bytes: 96,
    };
    let program = MaterialProgram::compile_with_limits(SOURCE, limits).unwrap();
    for rejected in [
        Scene3dMaterialLimits {
            max_source_bytes: SOURCE.len() - 1,
            ..limits
        },
        Scene3dMaterialLimits {
            max_resources: 4,
            ..limits
        },
        Scene3dMaterialLimits {
            max_uniform_bytes: 95,
            ..limits
        },
    ] {
        assert!(MaterialProgram::compile_with_limits(SOURCE, rejected).is_err());
    }
    let device = wgpu::Limits {
        max_bind_groups: 2,
        max_bindings_per_bind_group: 18,
        max_sampled_textures_per_shader_stage: 10,
        max_samplers_per_shader_stage: 8,
        max_uniform_buffers_per_shader_stage: 3,
        max_buffers_and_acceleration_structures_per_shader_stage: 3,
        ..Default::default()
    };
    program.validate_limits(&device).unwrap();
    for rejected in [
        wgpu::Limits {
            max_bind_groups: 1,
            ..device
        },
        wgpu::Limits {
            max_bindings_per_bind_group: 17,
            ..device
        },
        wgpu::Limits {
            max_sampled_textures_per_shader_stage: 9,
            ..device
        },
        wgpu::Limits {
            max_samplers_per_shader_stage: 7,
            ..device
        },
        wgpu::Limits {
            max_uniform_buffers_per_shader_stage: 2,
            ..device
        },
        wgpu::Limits {
            max_buffers_and_acceleration_structures_per_shader_stage: 2,
            ..device
        },
    ] {
        assert!(program.validate_limits(&rejected).is_err());
    }
    let large = MaterialProgram::compile(
        &SOURCE.replace("value: vec4<f32>", "value: array<vec4<f32>, 512>"),
    )
    .unwrap();
    assert!(
        large
            .validate_limits(&wgpu::Limits {
                max_uniform_buffer_binding_size: 4096,
                ..device
            })
            .is_err()
    );
}

#[test]
fn resource_free_materials_reserve_the_empty_extension_group() {
    let program = MaterialProgram::compile(DEFAULT).unwrap();
    let device = wgpu::Limits {
        max_bind_groups: 2,
        max_buffers_and_acceleration_structures_per_shader_stage: 1,
        ..Default::default()
    };
    program.validate_limits(&device).unwrap();
    let error = program
        .validate_limits(&wgpu::Limits {
            max_bind_groups: 1,
            ..device
        })
        .unwrap_err();
    assert!(error.to_string().contains("bind groups"), "{error}");
    assert!(
        program
            .validate_limits(&wgpu::Limits {
                max_buffers_and_acceleration_structures_per_shader_stage: 0,
                ..device
            })
            .is_err()
    );
}

#[test]
fn material_resources_reject_unsupported_and_overlapping_bindings() {
    for declaration in [
        "@group(1) @binding(18) var<storage, read> data: array<vec4<f32>>;",
        "@group(1) @binding(18) var<uniform> scalar: f32;",
        "struct Dynamic { data: array<vec4<f32>> } @group(1) @binding(18) var<uniform> dynamic: Dynamic;",
        "@group(1) @binding(18) var depth: texture_depth_2d;",
        "@group(1) @binding(18) var array_map: texture_2d_array<f32>;",
        "@group(1) @binding(18) var integer_map: texture_2d<u32>;",
        "@group(1) @binding(18) var multisampled_map: texture_multisampled_2d<f32>;",
        "@group(1) @binding(18) var comparison: sampler_comparison;",
        "@group(1) @binding(18) var writable: texture_storage_2d<rgba8unorm, write>;",
        "@group(1) @binding(7) var duplicate_map: texture_2d<f32>;",
    ] {
        assert!(
            MaterialProgram::compile(&format!("{SOURCE}\n{declaration}")).is_err(),
            "{declaration}"
        );
    }
}

#[test]
fn material_coverage_requires_explicit_sampling_even_through_helpers() {
    for sample in [
        "textureSample(surface_map, map_sampler, input.uv.xy)",
        "textureSampleBias(surface_map, map_sampler, input.uv.xy, 1.0)",
    ] {
        let source = SOURCE.replace(
            "textureSampleGrad(surface_map, map_sampler, input.uv.xy, gradients[0], gradients[1])",
            sample,
        );
        let error = MaterialProgram::compile(&source).unwrap_err();
        assert!(
            error.to_string().contains("explicit texture levels"),
            "{error:#}"
        );
    }
    MaterialProgram::compile(&SOURCE.replace(
        "textureSampleGrad(surface_map, map_sampler, input.uv.xy, gradients[0], gradients[1])",
        "textureSampleLevel(surface_map, map_sampler, input.uv.xy, 0.0)",
    ))
    .unwrap();
}
