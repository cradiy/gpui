use super::*;

#[test]
fn scene3d_mesh_pass_variants_cover_all_raster_controls_but_share_stage_pipelines() {
    let state = MeshPassState3d::default();
    let variants = [
        MeshPassState3d {
            cull: gpui::MeshPassCull3d::Front,
            ..state
        },
        MeshPassState3d {
            depth_compare: MeshPassDepth3d::Always,
            ..state
        },
        MeshPassState3d {
            depth_write: true,
            ..state
        },
        MeshPassState3d {
            depth_bias: -2,
            ..state
        },
        MeshPassState3d {
            depth_slope_bias: 0.25,
            ..state
        },
        MeshPassState3d {
            depth_bias_clamp: 0.5,
            ..state
        },
        MeshPassState3d {
            blend: MeshPassBlend3d::Additive,
            ..state
        },
        MeshPassState3d {
            alpha_mode: gpui::AlphaMode3d::Mask,
            ..state
        },
        MeshPassState3d {
            alpha_cutoff: 0.75,
            ..state
        },
    ];
    let keys: HashSet<_> = std::iter::once(state).chain(variants).map(key).collect();
    assert_eq!(keys.len(), 10);
    assert_eq!(
        key(state),
        key(MeshPassState3d {
            stage: gpui::MeshPassStage3d::AfterTransparent,
            ..state
        })
    );
    for state in variants {
        assert!(state.is_valid());
        let depth = depth(state);
        assert_eq!(depth.depth_write_enabled, Some(state.depth_write));
        assert_eq!(depth.bias.constant, state.depth_bias);
        assert_eq!(depth.bias.slope_scale, state.depth_slope_bias);
        assert_eq!(depth.bias.clamp, state.depth_bias_clamp);
    }
    assert_eq!(
        depth(variants[1]).depth_compare,
        Some(wgpu::CompareFunction::Always)
    );
    assert_eq!(constants(variants[0])[0], ("mesh_pass_cull", 1.));
    assert_eq!(constants(variants[7])[1], ("mesh_pass_alpha_mode", 1.));
    assert_eq!(constants(variants[8])[2], ("mesh_pass_alpha_cutoff", 0.75));
    assert!(blend(MeshPassBlend3d::Replace).is_none());
    let add = blend(MeshPassBlend3d::Additive).unwrap();
    assert_eq!(add.color.dst_factor, wgpu::BlendFactor::One);
    assert_eq!(
        add.alpha,
        wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING.alpha
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(
            !MeshPassState3d {
                depth_slope_bias: value,
                ..state
            }
            .is_valid()
        );
        assert!(
            !MeshPassState3d {
                depth_bias_clamp: value,
                ..state
            }
            .is_valid()
        );
        assert!(
            !MeshPassState3d {
                alpha_cutoff: value,
                ..state
            }
            .is_valid()
        );
    }
    assert!(
        !MeshPassState3d {
            depth_bias_clamp: -1.,
            ..state
        }
        .is_valid()
    );
    assert!(
        !MeshPassState3d {
            alpha_cutoff: 1.1,
            ..state
        }
        .is_valid()
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn scene3d_mesh_pass_color_is_independent_from_primary_data_coverage() -> Result<()> {
    use crate::wgpu_renderer::scene3d::tests::{frame, object};
    use crate::{
        Scene3dChannels, Scene3dMaterialBindingLimits, Scene3dMaterialProgram, Scene3dOutputConfig,
        WgpuContext, WgpuScene3dRenderer,
    };
    use std::sync::Arc;
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> { return vec4<f32>(0.0, 0.0, 1.0, 0.5); }
        fn material_shading(base: vec3<f32>, input: SurfaceInput, gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
    "#,
        )?,
    )?;
    let snapshot = source.bind([], Scene3dMaterialBindingLimits::default())?;
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    let mut object = object();
    object.color = gpui::rgb(0xff0000);
    object.unlit = true;
    object.model[3][2] = 0.5;
    let state = MeshPassState3d::default();
    for (state, expected) in [
        (state, [0.5, 0., 0.5, 1.]),
        (
            MeshPassState3d {
                blend: MeshPassBlend3d::Replace,
                ..state
            },
            [0., 0., 0.5, 0.5],
        ),
        (
            MeshPassState3d {
                blend: MeshPassBlend3d::Additive,
                ..state
            },
            [1., 0., 0.5, 1.],
        ),
        (
            MeshPassState3d {
                depth_compare: MeshPassDepth3d::Never,
                ..state
            },
            [1., 0., 0., 1.],
        ),
        (
            MeshPassState3d {
                alpha_mode: gpui::AlphaMode3d::Mask,
                alpha_cutoff: 0.75,
                ..state
            },
            [1., 0., 0., 1.],
        ),
    ] {
        object.mesh_passes = vec![gpui::MeshPass3d {
            expansion: None,
            material: gpui::MeshMaterial3d::new(Arc::new(snapshot.clone())),
            state,
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
        let mut pending = output.readback()?;
        context.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(std::time::Duration::from_secs(10)),
        })?;
        let pixels = pending
            .try_read()?
            .context("mesh pass readback not ready")?;
        let sample = 24 * 64 + 36;
        let rgba = pixels.linear_rgba.as_ref().unwrap()[sample];
        for (value, expected) in rgba.into_iter().zip(expected) {
            assert!((value - expected).abs() < 0.002, "{state:?}: {rgba:?}");
        }
        assert_eq!(pixels.object_ids.as_ref().unwrap()[sample], 1);
        assert!((pixels.linear_depth.as_ref().unwrap()[sample] - 0.5).abs() < 0.001);
        assert_eq!(pixels.world_normals.as_ref().unwrap()[sample][3], 1.);
    }
    Ok(())
}
