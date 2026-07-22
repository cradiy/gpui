mod cosmic_text_system;
mod wgpu_atlas;
mod wgpu_context;
mod wgpu_renderer;

pub use cosmic_text_system::*;
pub use wgpu;
pub use wgpu_atlas::*;
pub use wgpu_context::*;
pub use wgpu_renderer::{GpuContext, WgpuRenderer, WgpuSurfaceConfig};

#[cfg(test)]
mod tests {
    fn shader_struct_span(module: &wgpu::naga::Module, name: &str) -> u32 {
        module
            .types
            .iter()
            .find_map(|(_, ty)| {
                if ty.name.as_deref() != Some(name) {
                    return None;
                }
                match &ty.inner {
                    wgpu::naga::TypeInner::Struct { span, .. } => Some(*span),
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("missing shader struct {name}"))
    }

    fn shader_struct_offsets(module: &wgpu::naga::Module, name: &str) -> Vec<(String, u32)> {
        module
            .types
            .iter()
            .find_map(|(_, ty)| {
                if ty.name.as_deref() != Some(name) {
                    return None;
                }
                match &ty.inner {
                    wgpu::naga::TypeInner::Struct { members, .. } => Some(
                        members
                            .iter()
                            .map(|member| {
                                (
                                    member.name.as_deref().unwrap_or("?").to_owned(),
                                    member.offset,
                                )
                            })
                            .collect(),
                    ),
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("missing shader struct {name}"))
    }

    #[test]
    fn main_shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shaders.wgsl"))
            .expect("main shader should parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("main shader should validate");
    }

    #[test]
    fn effect_shader_contract_is_valid_wgsl() {
        let source = gpui::compose_effect_wgsl(
            r#"
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    return vec4<f32>(input.uv, params.slots[0].x, 1.0);
}
"#,
        );
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .expect("effect shader contract should parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("effect shader contract should validate");

        assert_eq!(
            shader_struct_span(&module, "EffectInstance") as usize,
            std::mem::size_of::<super::wgpu_renderer::EffectInstance>(),
        );
    }

    #[test]
    fn built_in_effect_shaders_are_valid_wgsl() {
        for shader in [
            gpui_effects::aurora_shader(),
            gpui_effects::plasma_shader(),
            gpui_effects::color_orbs_shader(),
            gpui_effects::album_glow_shader(),
            gpui_effects::flip_shader(),
            gpui_effects::rigid_flip_shader(),
            gpui_effects::soft_flip_shader(),
            gpui_effects::spectrum_mask_shader(),
        ] {
            let source = gpui::compose_effect_shader_wgsl(&shader);
            let module =
                wgpu::naga::front::wgsl::parse_str(&source).expect("built-in effect should parse");
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .expect("built-in effect should validate");
        }
    }

    fn assert_effect_translates_to_msl_and_hlsl(shader: gpui::EffectShader) {
        let source = gpui::compose_effect_shader_wgsl(&shader);
        let module = naga::front::wgsl::parse_str(&source).expect("effect should parse");
        let info = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("effect should validate");

        let mut resources = naga::back::msl::EntryPointResources::default();
        resources.resources.insert(
            naga::ResourceBinding {
                group: 0,
                binding: 0,
            },
            naga::back::msl::BindTarget {
                buffer: Some(0),
                ..Default::default()
            },
        );
        resources.resources.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 3,
            },
            naga::back::msl::BindTarget {
                texture: Some(1),
                ..Default::default()
            },
        );
        resources.resources.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 4,
            },
            naga::back::msl::BindTarget {
                texture: Some(2),
                ..Default::default()
            },
        );
        resources.resources.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 5,
            },
            naga::back::msl::BindTarget {
                texture: Some(3),
                ..Default::default()
            },
        );
        resources.resources.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 1,
            },
            naga::back::msl::BindTarget {
                texture: Some(0),
                ..Default::default()
            },
        );
        resources.sizes_buffer = Some(2);
        resources.resources.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 0,
            },
            naga::back::msl::BindTarget {
                buffer: Some(1),
                ..Default::default()
            },
        );
        let mut msl_options = naga::back::msl::Options {
            lang_version: (2, 0),
            fake_missing_bindings: false,
            ..Default::default()
        };
        msl_options
            .per_entry_point_map
            .insert("vs_effect".to_owned(), resources.clone());
        msl_options
            .per_entry_point_map
            .insert("fs_effect".to_owned(), resources);
        let (msl, msl_info) = naga::back::msl::write_string(
            &module,
            &info,
            &msl_options,
            &naga::back::msl::PipelineOptions::default(),
        )
        .expect("effect should translate to MSL");
        assert!(
            msl_info.entry_point_names.iter().all(Result::is_ok),
            "entry point translation failed: {:?}",
            msl_info.entry_point_names,
        );
        assert!(msl.contains("vertex"), "{msl}");
        assert!(msl.contains("fragment"), "{msl}");
        assert!(msl.contains("texture2d"), "{msl}");
        assert_eq!(msl_info.entry_point_names.len(), 2);
        let mut hlsl_options = naga::back::hlsl::Options {
            shader_model: naga::back::hlsl::ShaderModel::V5_0,
            fake_missing_bindings: false,
            ..Default::default()
        };
        hlsl_options.binding_map.insert(
            naga::ResourceBinding {
                group: 0,
                binding: 0,
            },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: 0,
                ..Default::default()
            },
        );
        hlsl_options.binding_map.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 3,
            },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: 2,
                ..Default::default()
            },
        );
        hlsl_options.binding_map.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 4,
            },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: 3,
                ..Default::default()
            },
        );
        hlsl_options.binding_map.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 5,
            },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: 4,
                ..Default::default()
            },
        );
        hlsl_options.binding_map.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 1,
            },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: 0,
                ..Default::default()
            },
        );
        hlsl_options.binding_map.insert(
            naga::ResourceBinding {
                group: 1,
                binding: 0,
            },
            naga::back::hlsl::BindTarget {
                space: 0,
                register: 1,
                ..Default::default()
            },
        );
        let mut hlsl = String::new();
        naga::back::hlsl::Writer::new(
            &mut hlsl,
            &hlsl_options,
            &naga::back::hlsl::PipelineOptions::default(),
        )
        .write(&module, &info, None)
        .expect("effect should translate to HLSL");
        assert!(hlsl.contains("vs_effect"));
        assert!(hlsl.contains("fs_effect"));
        assert!(hlsl.contains("Texture2D"), "{hlsl}");
    }

    #[test]
    fn built_in_effect_translates_to_msl_and_hlsl() {
        let four_image_shader = gpui::EffectShader::wgsl_four_images(
            r#"
fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> {
    return (
        sample_effect_image(input, input.uv)
        + sample_effect_second_image(input, input.uv)
        + sample_effect_third_image(input, input.uv)
        + sample_effect_fourth_image(input, input.uv)
    ) * 0.25;
}
"#,
        );
        for shader in [
            four_image_shader,
            gpui_effects::flip_shader(),
            gpui_effects::spectrum_mask_shader(),
        ] {
            assert_effect_translates_to_msl_and_hlsl(shader);
        }
    }

    #[test]
    fn shader_struct_layouts_match_rust() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shaders.wgsl"))
            .expect("main shader should parse");

        assert_eq!(
            shader_struct_span(&module, "Background") as usize,
            std::mem::size_of::<gpui::Background>()
        );
        assert_eq!(
            shader_struct_offsets(&module, "Quad"),
            vec![
                (
                    "order".into(),
                    std::mem::offset_of!(gpui::Quad, order) as u32
                ),
                (
                    "border_style".into(),
                    std::mem::offset_of!(gpui::Quad, border_style) as u32,
                ),
                (
                    "bounds".into(),
                    std::mem::offset_of!(gpui::Quad, bounds) as u32
                ),
                (
                    "content_mask".into(),
                    std::mem::offset_of!(gpui::Quad, content_mask) as u32,
                ),
                (
                    "background".into(),
                    std::mem::offset_of!(gpui::Quad, background) as u32,
                ),
                (
                    "border_colors".into(),
                    std::mem::offset_of!(gpui::Quad, border_colors) as u32,
                ),
                (
                    "border_gradient".into(),
                    std::mem::offset_of!(gpui::Quad, border_gradient) as u32,
                ),
                (
                    "corner_radii".into(),
                    std::mem::offset_of!(gpui::Quad, corner_radii) as u32,
                ),
                (
                    "border_widths".into(),
                    std::mem::offset_of!(gpui::Quad, border_widths) as u32,
                ),
            ]
        );
        assert_eq!(
            shader_struct_span(&module, "Quad") as usize,
            std::mem::size_of::<gpui::Quad>()
        );
        assert_eq!(
            shader_struct_span(&module, "PathRasterizationVertex") as usize,
            std::mem::size_of::<super::wgpu_renderer::PathRasterizationVertex>()
        );
        assert_eq!(
            shader_struct_span(&module, "SurfaceParams") as usize,
            std::mem::size_of::<super::wgpu_renderer::SurfaceParams>()
        );
    }

    #[test]
    fn limited_range_yuv_maps_reference_black_and_white() {
        let rows = super::wgpu_renderer::yuv_to_rgb_rows(gpui::SurfaceColorInfo {
            matrix: gpui::YuvMatrix::Bt709,
            range: gpui::ColorRange::Limited,
        });
        let convert = |y: f32, cb: f32, cr: f32| {
            rows.map(|row| row[0] * y + row[1] * cb + row[2] * cr + row[3])
        };

        let black = convert(16.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0);
        let white = convert(235.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0);
        for channel in black {
            assert!(channel.abs() < 1e-5, "black channel was {channel}");
        }
        for channel in white {
            assert!((channel - 1.0).abs() < 1e-5, "white channel was {channel}");
        }
    }

    #[test]
    fn bt601_and_bt709_use_different_chroma_coefficients() {
        let bt601 = super::wgpu_renderer::yuv_to_rgb_rows(gpui::SurfaceColorInfo {
            matrix: gpui::YuvMatrix::Bt601,
            range: gpui::ColorRange::Full,
        });
        let bt709 = super::wgpu_renderer::yuv_to_rgb_rows(gpui::SurfaceColorInfo {
            matrix: gpui::YuvMatrix::Bt709,
            range: gpui::ColorRange::Full,
        });

        assert_ne!(bt601, bt709);
        assert!((bt601[0][2] - 1.402).abs() < 1e-6);
        assert!((bt709[0][2] - 1.5748).abs() < 1e-6);
    }

    #[test]
    fn surface_cache_reuses_textures_and_only_uploads_new_sequences() {
        use super::wgpu_renderer::{SurfaceCacheAction, surface_cache_action};

        let size = gpui::size(gpui::DevicePixels(2), gpui::DevicePixels(2));
        let handle = gpui::SurfaceHandle::new();
        let first = gpui::SurfaceFrame::bgra(handle.clone(), 10, size, vec![0; 16], 8).unwrap();
        let next = gpui::SurfaceFrame::bgra(handle, 11, size, vec![1; 16], 8).unwrap();

        assert_eq!(
            surface_cache_action(None, &first),
            SurfaceCacheAction::Create
        );
        assert_eq!(
            surface_cache_action(Some((10, gpui::SurfaceFormat::Bgra8, size)), &first),
            SurfaceCacheAction::Reuse
        );
        assert_eq!(
            surface_cache_action(Some((10, gpui::SurfaceFormat::Bgra8, size)), &next),
            SurfaceCacheAction::Upload
        );
        assert_eq!(
            surface_cache_action(Some((10, gpui::SurfaceFormat::Rgba8, size)), &first),
            SurfaceCacheAction::Recreate
        );
        assert_eq!(
            surface_cache_action(
                Some((
                    10,
                    gpui::SurfaceFormat::Bgra8,
                    gpui::size(4.into(), 2.into())
                )),
                &first
            ),
            SurfaceCacheAction::Recreate
        );
    }

    #[test]
    fn visible_rect_is_normalized_to_texture_coordinates() {
        let coded_size = gpui::size(gpui::DevicePixels(8), gpui::DevicePixels(4));
        let frame = gpui::SurfaceFrame::new(
            gpui::SurfaceHandle::new(),
            0,
            coded_size,
            gpui::bounds(
                gpui::point(gpui::DevicePixels(2), gpui::DevicePixels(1)),
                gpui::size(gpui::DevicePixels(4), gpui::DevicePixels(2)),
            ),
            gpui::size(gpui::DevicePixels(4), gpui::DevicePixels(2)),
            gpui::SurfaceFormat::Rgba8,
            [gpui::SurfacePlane::new(vec![0; 8 * 4 * 4], 8 * 4)],
            gpui::SurfaceColorInfo::default(),
        )
        .unwrap();

        assert_eq!(
            super::wgpu_renderer::surface_uv_bounds(&frame),
            ([0.25, 0.25], [0.5, 0.5])
        );
    }
}
