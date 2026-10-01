use super::*;
use gpui::{BackdropSampling, BackdropShader, EffectShader};

#[test]
fn effect_translation_preserves_image_bindings() {
    let plain = "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return vec4<f32>(input.uv, params.slots[0].x, 1.0); }";
    let one = "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }";
    let two = "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv) + sample_effect_second_image(input, input.uv); }";
    let four = "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv) + sample_effect_second_image(input, input.uv) + sample_effect_third_image(input, input.uv) + sample_effect_fourth_image(input, input.uv); }";
    for shader in [
        EffectShader::wgsl(plain),
        EffectShader::wgsl_image(one),
        EffectShader::wgsl_mask(plain),
        EffectShader::wgsl_two_images(two),
        EffectShader::wgsl_four_images(four),
    ] {
        let source = gpui::compose_effect_shader_wgsl(&shader);
        let kind = ShaderKind::Effect {
            image_count: shader.image_count(),
        };
        let msl = to_msl(&source, kind).unwrap();
        let hlsl = to_hlsl(&source, kind).unwrap();
        for entry in kind.entries() {
            assert!(msl.contains(&format!(" {entry}(")));
            assert!(hlsl.contains(&format!(" {entry}(")));
        }
        assert!(msl.contains("[[buffer(1)]]"));
        assert!(hlsl.contains("register(t1)"));
        for index in 0..shader.image_count() {
            assert!(msl.contains(&format!("[[texture({index})]]")));
            let register = if index == 0 { 0 } else { index + 1 };
            assert!(hlsl.contains(&format!("register(t{register})")));
        }
        assert!(!hlsl.contains("SamplerState"));
    }
}

#[test]
fn backdrop_sampling_selects_native_resource_contracts() {
    let shader = BackdropShader::wgsl(
        "fn backdrop_effect(input: BackdropInput, params: BackdropParams) -> vec4<f32> { return mix(sample_raw_backdrop(input, input.pointer), sample_blurred_backdrop(input, -input.pointer), params.slots[0].x); }",
    );
    let hardware = gpui::compose_backdrop_shader_wgsl(&shader);
    let manual =
        gpui::compose_backdrop_shader_wgsl_with_sampling(&shader, BackdropSampling::Manual);
    let msl = to_msl(&hardware, ShaderKind::Backdrop).unwrap();
    let hlsl = to_hlsl(&manual, ShaderKind::Backdrop).unwrap();
    assert!(msl.contains("[[sampler(0)]]"));
    assert!(msl.contains("[[texture(0)]]"));
    assert!(msl.contains("[[texture(1)]]"));
    assert!(!hlsl.contains("SamplerState"));
    assert!(hlsl.contains("register(t0)"));
    assert!(hlsl.contains("register(t2)"));
    let module = naga::front::wgsl::parse_str(&manual).unwrap();
    assert!(!module.global_variables.iter().any(|(_, v)| v.binding
        == Some(naga::ResourceBinding {
            group: 1,
            binding: 2
        })));

    let blur = to_hlsl(crate::BACKDROP_BLUR_MANUAL_WGSL, ShaderKind::BackdropBlur).unwrap();
    assert!(blur.contains(" fs_blur("));
    assert!(!blur.contains("SamplerState"));
}

#[test]
fn invalid_native_contracts_return_errors() {
    let kind = ShaderKind::Effect { image_count: 0 };
    let source = gpui::compose_effect_wgsl(
        "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return vec4<f32>(1.0); }",
    );
    for translate in [to_msl, to_hlsl] {
        assert!(translate("invalid WGSL", kind).is_err());
        assert!(translate(&source, ShaderKind::Effect { image_count: 3 }).is_err());
        assert!(translate(&source, ShaderKind::Backdrop).is_err());
        let source = source.replace("return vec4<f32>(1.0);", "return extra.value;");
        let source = format!(
            "struct Extra {{ value: vec4<f32> }}; @group(3) @binding(7) var<uniform> extra: Extra;\n{source}"
        );
        parse(&source, kind).expect("unmapped resource shader must be otherwise valid");
        assert!(
            translate(&source, kind).is_err(),
            "unmapped resources must fail translation"
        );
    }
}
