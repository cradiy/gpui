use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue,
    Scene3dOutputConfig, WgpuContext, WgpuScene3dRenderer,
};
use anyhow::{Context as _, Result};
use gpui::{
    MeshPass3d, MeshPassCull3d as Cull, MeshPassDepth3d as Depth, MeshPassStage3d as Stage,
    MeshPassState3d as State,
};
use std::sync::Arc;

#[test]
#[ignore = "requires a GPU adapter"]
fn mesh_pass_raster_controls_order_and_depth_writes_preserve_primary_channels() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        struct Tint { color: vec4<f32> }
        @group(1) @binding(0) var<uniform> tint: Tint;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> { return tint.color; }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
    "#,
        )?,
    )?;
    let material = |color: [f32; 4]| -> Result<_> {
        Ok(gpui::MeshMaterial3d::new(Arc::new(source.bind(
            [(
                0,
                Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&color).into()),
            )],
            Default::default(),
        )?)))
    };
    let blue = material([0., 0., 1., 0.5])?;
    let green = material([0., 1., 0., 1.])?;
    let yellow = material([1., 1., 0., 0.5])?;
    let pass = |material: &gpui::MeshMaterial3d, state| MeshPass3d {
        material: material.clone(),
        state,
        expansion: None,
    };
    let mut base = object();
    base.color = gpui::rgb(0xff0000);
    base.model[3][2] = 0.5;
    let mut cases = Vec::new();
    for (compare, bias, visible) in [
        (Depth::Never, 0, false),
        (Depth::Less, 0, false),
        (Depth::Equal, 0, true),
        (Depth::LessEqual, 0, true),
        (Depth::Greater, 0, false),
        (Depth::NotEqual, 0, false),
        (Depth::GreaterEqual, 0, true),
        (Depth::Always, 0, true),
        (Depth::Less, -2, true),
        (Depth::Greater, 2, true),
        (Depth::Less, 2, false),
        (Depth::Greater, -2, false),
    ] {
        let mut object = base.clone();
        object.mesh_passes = vec![pass(
            &blue,
            State {
                depth_compare: compare,
                depth_bias: bias,
                ..Default::default()
            },
        )]
        .into();
        let expected = if visible {
            [0.5, 0., 0.5, 1.]
        } else {
            [1., 0., 0., 1.]
        };
        cases.push((
            format!("depth {compare:?}, bias {bias}"),
            vec![object],
            36,
            expected,
            1,
            0.5,
        ));
    }
    for write in [false, true] {
        let mut object = base.clone();
        object.mesh_passes = vec![
            pass(
                &blue,
                State {
                    depth_compare: Depth::Less,
                    depth_bias: -2,
                    depth_write: write,
                    ..Default::default()
                },
            ),
            pass(
                &green,
                State {
                    depth_compare: Depth::Greater,
                    ..Default::default()
                },
            ),
        ]
        .into();
        let expected = if write {
            [0., 1., 0., 1.]
        } else {
            [0.5, 0., 0.5, 1.]
        };
        cases.push((
            format!("depth write {write}"),
            vec![object],
            36,
            expected,
            1,
            0.5,
        ));
    }
    for sign in [-1., 1.] {
        for clamp in [0., -0.001, 0.001] {
            let mut object = base.clone();
            object.model[0][2] = 0.25;
            object.mesh_passes = vec![
                pass(
                    &blue,
                    State {
                        depth_compare: Depth::Always,
                        depth_slope_bias: sign,
                        depth_bias_clamp: clamp,
                        depth_write: true,
                        ..Default::default()
                    },
                ),
                pass(
                    &green,
                    State {
                        depth_compare: if sign < 0. {
                            Depth::Less
                        } else {
                            Depth::Greater
                        },
                        depth_slope_bias: sign * 0.5,
                        ..Default::default()
                    },
                ),
            ]
            .into();
            let expected = if sign * clamp > 0. {
                [0., 1., 0., 1.]
            } else {
                [0.5, 0., 0.5, 1.]
            };
            cases.push((
                format!("slope {sign}, clamp {clamp}"),
                vec![object],
                36,
                expected,
                1,
                0.53515625,
            ));
        }
    }
    for mirrored in [false, true] {
        for cull in [Cull::None, Cull::Front, Cull::Back] {
            let mut object = base.clone();
            if mirrored {
                object.model[0][0] = -1.;
                object.normal[0][0] = -1.;
            }
            object.mesh_passes = vec![pass(
                &blue,
                State {
                    cull,
                    ..Default::default()
                },
            )]
            .into();
            let expected = if cull == Cull::Front {
                [1., 0., 0., 1.]
            } else {
                [0.5, 0., 0.5, 1.]
            };
            cases.push((
                format!("mirror {mirrored}, cull {cull:?}"),
                vec![object],
                if mirrored { 27 } else { 36 },
                expected,
                1,
                0.5,
            ));
        }
    }
    for stage in [Stage::AfterOpaque, Stage::AfterTransparent] {
        let mut object = base.clone();
        object.mesh_passes = vec![pass(
            &blue,
            State {
                stage,
                ..Default::default()
            },
        )]
        .into();
        let mut front = base.clone();
        front.color = gpui::Rgba {
            r: 0.,
            g: 1.,
            b: 0.,
            a: 0.5,
        };
        front.alpha_mode = gpui::AlphaMode3d::Blend;
        front.model[3][2] = 0.4;
        front.output_id = 2;
        let expected = if stage == Stage::AfterOpaque {
            [0.25, 0.5, 0.25, 1.]
        } else {
            [0.25, 0.25, 0.5, 1.]
        };
        cases.push((
            format!("stage {stage:?}"),
            vec![object, front],
            36,
            expected,
            2,
            0.4,
        ));
    }
    for reverse in [false, true] {
        let mut object = base.clone();
        let mut passes = vec![
            pass(&blue, State::default()),
            pass(&yellow, State::default()),
        ];
        if reverse {
            passes.reverse();
        }
        object.mesh_passes = passes.into();
        let expected = if reverse {
            [0.5, 0.25, 0.5, 1.]
        } else {
            [0.75, 0.5, 0.25, 1.]
        };
        cases.push((
            format!("declaration order reversed {reverse}"),
            vec![object],
            36,
            expected,
            1,
            0.5,
        ));
    }
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    for samples in [1, 4] {
        for (label, objects, x, expected, id, depth) in &cases {
            let mut input = frame(objects);
            input.world_to_view[2][2] = -1.;
            let output = renderer.render(
                &input,
                Scene3dOutputConfig {
                    size: [64, 64],
                    channels: Scene3dChannels::all(),
                    color_samples: samples,
                },
            )?;
            let mut pending = output.readback()?;
            context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(15)),
            })?;
            let pixels = pending
                .try_read()?
                .context("mesh pass readback not ready")?;
            let index = 24 * 64 + x;
            let color = pixels.linear_rgba.as_ref().unwrap()[index];
            for (actual, expected) in color.iter().zip(expected) {
                assert!(
                    (actual - expected).abs() < 0.002,
                    "{label}, samples {samples}: {color:?} != {expected}"
                );
            }
            assert_eq!(pixels.object_ids.as_ref().unwrap()[index], *id, "{label}");
            assert!(
                (pixels.linear_depth.as_ref().unwrap()[index] - depth).abs() < 0.001,
                "{label}"
            );
            assert_eq!(
                pixels.world_normals.as_ref().unwrap()[index],
                [0., 0., 1., 1.],
                "{label}"
            );
        }
    }
    Ok(())
}
