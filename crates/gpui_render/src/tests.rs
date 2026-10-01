use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn manual_backdrop_blur_translates_without_sampler_bindings() {
    let module = naga::front::wgsl::parse_str(BACKDROP_BLUR_MANUAL_WGSL).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let mut options = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_0,
        fake_missing_bindings: false,
        ..Default::default()
    };
    for (group, binding, register) in [(0, 0, 0), (1, 0, 1), (1, 1, 0)] {
        options.binding_map.insert(
            naga::ResourceBinding { group, binding },
            naga::back::hlsl::BindTarget {
                space: 0,
                register,
                ..Default::default()
            },
        );
    }
    let pipeline_options = naga::back::hlsl::PipelineOptions::default();
    let mut output = String::new();
    let reflection = naga::back::hlsl::Writer::new(&mut output, &options, &pipeline_options)
        .write(&module, &info, None)
        .unwrap();
    let entries = reflection
        .entry_point_names
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(entries.iter().any(|entry| entry == "vs_backdrop"));
    assert!(entries.iter().any(|entry| entry == "fs_blur"));
    assert!(!output.contains("SamplerState"));
}

#[test]
fn effect_shaders_match_shared_instance_layouts() {
    let effect = gpui::compose_effect_wgsl(
        "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return vec4<f32>(input.uv, 0.0, 1.0); }",
    );
    let backdrop = gpui::compose_backdrop_shader_wgsl(&gpui::BackdropShader::wgsl(
        "fn backdrop_effect(input: BackdropInput, params: BackdropParams) -> vec4<f32> { return sample_blurred_backdrop(input, vec2<f32>(0.0)); }",
    ));
    let effect_offsets = vec![
        offset_of!(gpui::EffectInstance, bounds),
        offset_of!(gpui::EffectInstance, effect_bounds),
        offset_of!(gpui::EffectInstance, transformation),
        offset_of!(gpui::EffectInstance, content_mask),
        offset_of!(gpui::EffectInstance, corner_radii),
        offset_of!(gpui::EffectInstance, image_bounds),
        offset_of!(gpui::EffectInstance, second_image_bounds),
        offset_of!(gpui::EffectInstance, third_image_bounds),
        offset_of!(gpui::EffectInstance, fourth_image_bounds),
        offset_of!(gpui::EffectInstance, opacity),
        offset_of!(gpui::EffectInstance, time),
        offset_of!(gpui::EffectInstance, pad),
        offset_of!(gpui::EffectInstance, alignment_pad),
        offset_of!(gpui::EffectInstance, uniforms),
    ];
    let backdrop_offsets = vec![
        offset_of!(gpui::BackdropInstance, bounds),
        offset_of!(gpui::BackdropInstance, content_mask),
        offset_of!(gpui::BackdropInstance, corner_radii),
        offset_of!(gpui::BackdropInstance, blur_radius),
        offset_of!(gpui::BackdropInstance, opacity),
        offset_of!(gpui::BackdropInstance, time),
        offset_of!(gpui::BackdropInstance, pointer_active),
        offset_of!(gpui::BackdropInstance, direction),
        offset_of!(gpui::BackdropInstance, pointer),
        offset_of!(gpui::BackdropInstance, uniforms),
    ];
    for (source, name, size, offsets) in [
        (
            effect.as_str(),
            "EffectInstance",
            size_of::<gpui::EffectInstance>(),
            &effect_offsets,
        ),
        (
            backdrop.as_str(),
            "BackdropInstance",
            size_of::<gpui::BackdropInstance>(),
            &backdrop_offsets,
        ),
        (
            BACKDROP_BLUR_WGSL,
            "BackdropInstance",
            size_of::<gpui::BackdropInstance>(),
            &backdrop_offsets,
        ),
        (
            BACKDROP_BLUR_MANUAL_WGSL,
            "BackdropInstance",
            size_of::<gpui::BackdropInstance>(),
            &backdrop_offsets,
        ),
    ] {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let ty = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap()
            .1;
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("not a struct")
        };
        assert_eq!(*span as usize, size, "{name} size");
        assert_eq!(
            &members
                .iter()
                .map(|member| member.offset as usize)
                .collect::<Vec<_>>(),
            offsets,
            "{name} offsets"
        );
    }
}

#[test]
fn primitive_shaders_match_host_layout_and_resource_contract() {
    let module = naga::front::wgsl::parse_str(&format!(
        "enable dual_source_blending;\n{}\n{SUBPIXEL_WGSL}",
        compose_shader("")
    ))
    .unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    for (name, size, offsets) in [
        (
            "PathRasterizationVertex",
            size_of::<gpui::PathRasterizationVertex>(),
            vec![
                offset_of!(gpui::PathRasterizationVertex, xy_position),
                offset_of!(gpui::PathRasterizationVertex, st_position),
                offset_of!(gpui::PathRasterizationVertex, color),
                offset_of!(gpui::PathRasterizationVertex, bounds),
            ],
        ),
        (
            "PathSprite",
            size_of::<gpui::PathSprite>(),
            vec![offset_of!(gpui::PathSprite, bounds)],
        ),
        (
            "SurfaceParams",
            size_of::<SurfaceParams>(),
            vec![
                offset_of!(SurfaceParams, bounds),
                offset_of!(SurfaceParams, clip_bounds),
                offset_of!(SurfaceParams, content_mask),
                offset_of!(SurfaceParams, uv_bounds),
                offset_of!(SurfaceParams, corner_radii),
                offset_of!(SurfaceParams, color_rows),
                offset_of!(SurfaceParams, opacity),
                offset_of!(SurfaceParams, _pad),
            ],
        ),
        (
            "GammaParams",
            size_of::<GammaParams>(),
            vec![
                offset_of!(GammaParams, gamma_ratios),
                offset_of!(GammaParams, grayscale_enhanced_contrast),
                offset_of!(GammaParams, subpixel_enhanced_contrast),
                offset_of!(GammaParams, is_bgr),
                offset_of!(GammaParams, _pad),
            ],
        ),
        (
            "MonochromeSprite",
            size_of::<gpui::MonochromeSprite>(),
            vec![
                offset_of!(gpui::MonochromeSprite, order),
                offset_of!(gpui::MonochromeSprite, pad),
                offset_of!(gpui::MonochromeSprite, bounds),
                offset_of!(gpui::MonochromeSprite, content_mask),
                offset_of!(gpui::MonochromeSprite, background),
                offset_of!(gpui::MonochromeSprite, background_bounds),
                offset_of!(gpui::MonochromeSprite, tile),
                offset_of!(gpui::MonochromeSprite, transformation),
            ],
        ),
        (
            "SubpixelSprite",
            size_of::<gpui::SubpixelSprite>(),
            vec![
                offset_of!(gpui::SubpixelSprite, order),
                offset_of!(gpui::SubpixelSprite, pad),
                offset_of!(gpui::SubpixelSprite, bounds),
                offset_of!(gpui::SubpixelSprite, content_mask),
                offset_of!(gpui::SubpixelSprite, background),
                offset_of!(gpui::SubpixelSprite, background_bounds),
                offset_of!(gpui::SubpixelSprite, tile),
                offset_of!(gpui::SubpixelSprite, transformation),
            ],
        ),
        (
            "GlobalParams",
            size_of::<PrimitiveGlobals>(),
            vec![
                offset_of!(PrimitiveGlobals, viewport_size),
                offset_of!(PrimitiveGlobals, premultiplied_alpha),
                offset_of!(PrimitiveGlobals, pad),
                offset_of!(PrimitiveGlobals, viewport_origin),
                offset_of!(PrimitiveGlobals, origin_pad),
            ],
        ),
        (
            "Quad",
            size_of::<gpui::Quad>(),
            vec![
                offset_of!(gpui::Quad, order),
                offset_of!(gpui::Quad, border_style),
                offset_of!(gpui::Quad, bounds),
                offset_of!(gpui::Quad, content_mask),
                offset_of!(gpui::Quad, background),
                offset_of!(gpui::Quad, border_colors),
                offset_of!(gpui::Quad, border_gradient),
                offset_of!(gpui::Quad, corner_radii),
                offset_of!(gpui::Quad, border_widths),
            ],
        ),
        (
            "Shadow",
            size_of::<gpui::Shadow>(),
            vec![
                offset_of!(gpui::Shadow, order),
                offset_of!(gpui::Shadow, blur_radius),
                offset_of!(gpui::Shadow, bounds),
                offset_of!(gpui::Shadow, corner_radii),
                offset_of!(gpui::Shadow, content_mask),
                offset_of!(gpui::Shadow, color),
                offset_of!(gpui::Shadow, element_bounds),
                offset_of!(gpui::Shadow, element_corner_radii),
                offset_of!(gpui::Shadow, inset),
                offset_of!(gpui::Shadow, pad),
            ],
        ),
        (
            "Underline",
            size_of::<gpui::Underline>(),
            vec![
                offset_of!(gpui::Underline, order),
                offset_of!(gpui::Underline, pad),
                offset_of!(gpui::Underline, bounds),
                offset_of!(gpui::Underline, content_mask),
                offset_of!(gpui::Underline, color),
                offset_of!(gpui::Underline, thickness),
                offset_of!(gpui::Underline, wavy),
            ],
        ),
        (
            "PolychromeSprite",
            size_of::<gpui::PolychromeSprite>(),
            vec![
                offset_of!(gpui::PolychromeSprite, order),
                offset_of!(gpui::PolychromeSprite, pad),
                offset_of!(gpui::PolychromeSprite, grayscale),
                offset_of!(gpui::PolychromeSprite, opacity),
                offset_of!(gpui::PolychromeSprite, bounds),
                offset_of!(gpui::PolychromeSprite, clip_bounds),
                offset_of!(gpui::PolychromeSprite, content_mask),
                offset_of!(gpui::PolychromeSprite, corner_radii),
                offset_of!(gpui::PolychromeSprite, tile),
                offset_of!(gpui::PolychromeSprite, transformation),
            ],
        ),
    ] {
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap();
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("not a struct")
        };
        assert_eq!(*span as usize, size, "{name} size");
        assert_eq!(
            members
                .iter()
                .map(|member| member.offset as usize)
                .collect::<Vec<_>>(),
            offsets,
            "{name} offsets"
        );
    }
    for (name, size) in [
        ("Background", size_of::<gpui::Background>()),
        ("BorderGradient", size_of::<gpui::BorderGradient>()),
    ] {
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap();
        let naga::TypeInner::Struct { span, .. } = ty.inner else {
            panic!("not a struct")
        };
        assert_eq!(span as usize, size, "{name}");
    }
    for (name, group, binding) in [
        ("globals", 0, 0),
        ("gamma_params", 0, 1),
        ("b_quads", 1, 0),
        ("b_shadows", 1, 0),
        ("b_underlines", 1, 0),
        ("b_path_vertices", 1, 0),
        ("b_path_sprites", 1, 0),
        ("b_poly_sprites", 1, 0),
        ("b_mono_sprites", 1, 0),
        ("b_subpixel_sprites", 1, 0),
        ("surface_locals", 1, 0),
        ("t_surface_0", 1, 1),
        ("t_surface_1", 1, 2),
        ("s_surface", 1, 3),
        ("t_sprite", 1, 1),
        ("s_sprite", 1, 2),
    ] {
        let (_, variable) = module
            .global_variables
            .iter()
            .find(|(_, variable)| variable.name.as_deref() == Some(name))
            .unwrap();
        assert_eq!(
            variable.binding,
            Some(naga::ResourceBinding { group, binding })
        );
    }
}
