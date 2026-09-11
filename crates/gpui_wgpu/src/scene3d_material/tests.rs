use super::*;

const SURFACE: &str = "fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> { return builtin_surface(input, gradients); }";
const SHADING: &str = "fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }";

#[test]
fn material_program_compiles_custom_light_response_with_shared_coverage() {
    let program = MaterialProgram::compile(&format!(r#"
        {SURFACE}
        fn band(value: f32) -> f32 {{ return floor(clamp(value, 0.0, 1.0) * 3.0) / 3.0; }}
        fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {{
            let normal = unit_vector(input.normal) * face_sign;
            var light = material_ambient(normal);
            for (var i = 0u; i < material_light_count(); i++) {{
                let direct = material_light(i, input.world, normal, gradients.shadow_depth);
                light += direct.energy * band(dot(normal, direct.direction));
            }}
            let view = material_view_direction(input.world);
            let eye_position = material_view_position(input.world);
            let eye_vector = material_view_vector(view);
            return base * light + vec3<f32>(pow(1.0 - abs(dot(view, normal)), 3.0) * 0.1)
                + abs(eye_vector) / (1.0 + length(eye_position));
        }}
    "#)).unwrap();
    assert!(program.source().contains("fn band"));
    assert!(!program.source().contains(DEFAULT));
    MaterialProgram::compile(DEFAULT).unwrap();
}

#[test]
fn material_program_rejects_renderer_state_and_entry_point_overrides() {
    for (extra, expected) in [
        ("var<private> state: f32;", "globals"),
        (
            "@group(2) @binding(0) var image_extra: texture_2d<f32>;",
            "globals",
        ),
        ("override threshold: f32 = 0.5;", "overrides"),
        ("@compute @workgroup_size(1) fn extra() {}", "entry points"),
        (
            "fn hidden() -> f32 { return params.ambient.x; }",
            "private renderer global",
        ),
        ("fn hidden() -> f32 { discard; return 0.0; }", "may discard"),
        (
            "fn hidden(value: f32) -> f32 { if (value > 0.0) { discard; } return value; }",
            "may discard",
        ),
        (
            "fn hidden(value: f32) -> f32 { loop { if (value > 0.0) { break; } discard; } return value; }",
            "may discard",
        ),
        (
            "fn hidden(input: Output) -> SurfaceGradients { return surface_gradients(input); }",
            "private renderer function",
        ),
    ] {
        let error = MaterialProgram::compile(&format!("{SURFACE}\n{SHADING}\n{extra}"))
            .err()
            .unwrap_or_else(|| panic!("invalid material accepted: {extra}"));
        assert!(error.to_string().contains(expected), "{extra}: {error:#}");
    }
    let oversized = " ".repeat(Scene3dMaterialLimits::default().max_source_bytes + 1);
    assert!(
        MaterialProgram::compile(&oversized)
            .err()
            .unwrap()
            .to_string()
            .contains("byte limit")
    );
    assert!(MaterialProgram::compile(SURFACE).is_err());
    assert!(MaterialProgram::compile(SHADING).is_err());
    assert!(
        MaterialProgram::compile(&format!(
            "{SURFACE}\n{}",
            SHADING.replace("return base;", "return vec4<f32>(base, 1.0);")
        ))
        .is_err()
    );
}

#[test]
fn material_program_checks_transitive_coverage_calls_inside_control_flow() {
    for body in [
        "if (input.world.x > 0.0) { return material_view_direction(input.world); } return vec3<f32>(0.0);",
        "return material_view_position(input.world);",
        "return material_view_vector(input.normal);",
        "switch (u32(input.color.x)) { case 0u: { return material_ambient(input.normal); } default: { return vec3<f32>(0.0); } }",
        "loop { if (input.color.x > 0.0) { break; } return material_ambient(input.normal); } return vec3<f32>(0.0);",
        "return vec3<f32>(dpdx(input.world.x));",
    ] {
        let source = format!(
            r#"
            fn hidden(input: SurfaceInput) -> vec3<f32> {{ {body} }}
            fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {{
                return vec4<f32>(hidden(input), 1.0);
            }}
            {SHADING}
        "#
        );
        let error = MaterialProgram::compile(&source)
            .err()
            .expect("view-dependent coverage accepted");
        assert!(
            error.to_string().contains("cannot call")
                || error.to_string().contains("supplied gradients"),
            "{error:#}"
        );
    }
}
