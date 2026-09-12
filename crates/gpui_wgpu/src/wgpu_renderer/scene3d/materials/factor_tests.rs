use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue,
    Scene3dOutputConfig, WgpuContext, WgpuScene3dGeometry, WgpuScene3dRenderer,
};
use anyhow::{Context as _, Result};
use gpui::PlatformAtlas as _;
use std::{borrow::Cow, collections::HashMap, sync::Arc};

fn decode(value: u8) -> f32 {
    let value = value as f32 / 255.;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn standard_factors_and_normal_maps_match_primary_and_additional_programs() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    let texels = [
        [[31_u8, 128, 192], [223, 64, 96]],
        [[128, 64, 32], [32, 192, 128]],
        [[192, 64, 255], [64, 192, 224]],
        [[64, 230, 17], [192, 8, 224]],
    ];
    let mut tiles = Vec::new();
    for (index, pixels) in texels.iter().enumerate() {
        let bytes: Vec<_> = pixels
            .iter()
            .flat_map(|&[r, g, b]| [b, g, r, 255])
            .collect();
        tiles.push(
            renderer
                .sprite_atlas()
                .get_or_insert_with(
                    &gpui::RenderImageParams {
                        image_id: gpui::ImageId(89020 + index),
                        frame_index: 0,
                    }
                    .into(),
                    &mut || {
                        Ok(Some((
                            gpui::size(gpui::DevicePixels(2), gpui::DevicePixels(1)),
                            Cow::Borrowed(&bytes),
                        )))
                    },
                )?
                .unwrap(),
        );
    }
    let source = Scene3dMaterialSource::new(
        context.clone(),
        Scene3dMaterialProgram::compile(
            r#"
        struct Probe { mode: vec4<u32> }
        @group(1) @binding(0) var<uniform> probe: Probe;
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            return builtin_surface(input, gradients);
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
            let factors = material_factors(input, gradients);
            switch probe.mode.x {
                case 0u: { return vec3<f32>(factors.metallic, factors.roughness, factors.occlusion); }
                case 1u: { return factors.emission; }
                case 2u: { return surface_normal(input, gradients.normal) * face_sign * 0.5 + vec3<f32>(0.5); }
                default: { return builtin_shading(base, input, gradients, face_sign); }
            }
        }
    "#,
        )?,
    )?;
    let mut snapshots = Vec::new();
    for mode in 0..4_u32 {
        snapshots.push(source.bind(
            [(
                0,
                Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&[mode, 0, 0, 0]).into()),
            )],
            Default::default(),
        )?);
    }
    let pbr = gpui::PbrMaterial3d {
        metallic: 0.7,
        roughness: 0.8,
        emissive: [0.4, 0.6, 1.2],
    };
    for (configured, maps, normal_scale, ao, handedness) in [
        (true, true, 1., 1., 1.),
        (true, true, 0.35, 0.4, -1.),
        (false, true, 1., 1., 1.),
        (true, false, 1., 1., 1.),
        (true, true, 0., 0., 1.),
    ] {
        let mut draw = object();
        draw.model[3][2] = 0.5;
        draw.pbr = configured.then_some(pbr);
        draw.normal_scale = normal_scale;
        draw.occlusion_strength = ao;
        let swapped = usize::from(handedness < 0.);
        let selections = [swapped, 1 - swapped, swapped, 1 - swapped];
        for (index, selection) in selections.into_iter().enumerate() {
            draw.mesh = draw.mesh.with_uv_set(
                (index + 1) as u32,
                vec![[0.25 + selection as f32 * 0.5, 0.5]; 3],
            )?;
        }
        draw.mesh = draw
            .mesh
            .with_tangents_for_uv_set(3, vec![[1., 0., 0., handedness]; 3])?;
        let map = |index: usize| {
            maps.then_some(gpui::MaterialTexture3d {
                tile: tiles[index],
                sampling: gpui::TextureSampling3d {
                    filter: gpui::TextureFilter3d::Nearest,
                    ..Default::default()
                },
                uv_set: (index + 1) as u32,
            })
        };
        draw.metallic_roughness_texture = map(0);
        draw.emissive_texture = map(1);
        draw.normal_texture = map(2);
        draw.occlusion_texture = map(3);
        let factors = draw.pbr.unwrap_or_default();
        let sampled = |index: usize| {
            if maps {
                texels[index][selections[index]].map(|v| v as f32 / 255.)
            } else {
                [1.; 3]
            }
        };
        let mr = sampled(0);
        let emission: [f32; 3] = std::array::from_fn(|axis| {
            factors.emissive[axis]
                * if maps {
                    decode(texels[1][selections[1]][axis])
                } else {
                    1.
                }
        });
        let mut normal = [0., 0., 1.];
        if maps && normal_scale > 0. {
            normal = sampled(2).map(|value| value * 2. - 1.);
            normal[0] *= normal_scale;
            normal[1] *= normal_scale * handedness;
            let length = normal.iter().map(|v| v * v).sum::<f32>().sqrt();
            normal = normal.map(|value| value / length);
        }
        let expected = [
            [
                factors.metallic * mr[2],
                factors.roughness * mr[1],
                1. + (sampled(3)[0] - 1.) * ao,
            ],
            emission,
            normal.map(|value| value * 0.5 + 0.5),
            [1.; 3],
        ];
        let records: Vec<_> = draw
            .mesh
            .vertices()
            .iter()
            .map(|vertex| {
                let mut record = [0_f32; 16];
                record[..3].copy_from_slice(&vertex.position);
                record[4..7].copy_from_slice(&vertex.normal);
                record[8] = 1.;
                record[11] = handedness;
                record
            })
            .collect();
        let deformation = context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let mut geometries = HashMap::new();
        for gpu in [false, true] {
            for samples in [1, 4] {
                for (additional, mode) in [
                    (false, 4),
                    (false, 0),
                    (true, 1),
                    (false, 2),
                    (true, 0),
                    (false, 1),
                    (true, 2),
                    (false, 3),
                    (false, 4),
                ] {
                    let mut draw = draw.clone();
                    let material = snapshots
                        .get(mode)
                        .map(|snapshot| gpui::MeshMaterial3d::new(Arc::new(snapshot.clone())));
                    if additional {
                        draw.mesh_passes = vec![gpui::MeshPass3d {
                            expansion: None,
                            material: material.unwrap(),
                            state: gpui::MeshPassState3d {
                                blend: gpui::MeshPassBlend3d::Replace,
                                alpha_mode: gpui::AlphaMode3d::Opaque,
                                ..Default::default()
                            },
                        }]
                        .into();
                    } else {
                        draw.custom_material = material;
                    }
                    if gpu {
                        let geometry = match geometries.entry(draw.texture_uv_sets()) {
                            std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                let geometry = WgpuScene3dGeometry::new(
                                    context.clone(),
                                    draw.mesh.clone(),
                                    *entry.key(),
                                    None,
                                )?
                                .evaluate(&deformation)?;
                                entry.insert(Arc::new(geometry))
                            }
                        };
                        draw.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(geometry.clone()));
                        draw.render_bounds = Some([[0.; 3], [1., 1., 0.]]);
                    }
                    let mut input = frame(&[draw]);
                    input.world_to_view[2][2] = -1.;
                    let output = renderer.render(
                        &input,
                        Scene3dOutputConfig {
                            size: [32, 32],
                            channels: Scene3dChannels::all(),
                            color_samples: samples,
                        },
                    )?;
                    let mut pending = output.readback()?;
                    context.device.poll(wgpu::PollType::Wait {
                        submission_index: None,
                        timeout: Some(std::time::Duration::from_secs(15)),
                    })?;
                    let pixels = pending.try_read()?.context("factor readback not ready")?;
                    let index = 12 * 32 + 18;
                    let expected = expected[mode.min(3)];
                    let actual = pixels.linear_rgba.as_ref().unwrap()[index];
                    for (a, b) in actual[..3].iter().zip(expected) {
                        assert!(
                            (a - b).abs() < 0.002,
                            "configured {configured}, maps {maps}, scale {normal_scale}, hand {handedness}, gpu {gpu}, samples {samples}, additional {additional}, mode {mode}: {actual:?} != {expected:?}"
                        );
                    }
                    assert_eq!(actual[3], 1.);
                    assert_eq!(pixels.object_ids.as_ref().unwrap()[index], 1);
                    assert_eq!(pixels.linear_depth.as_ref().unwrap()[index], 0.5);
                    assert_eq!(
                        pixels.world_normals.as_ref().unwrap()[index],
                        [0., 0., 1., 1.]
                    );
                }
            }
        }
    }
    Ok(())
}
