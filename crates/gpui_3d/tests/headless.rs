#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use gpui::rgb;
use gpui_3d::{Camera, HeadlessRenderer, Material, Mesh, Node, Scene3dOutputConfig, SceneGraph};

#[test]
#[ignore = "requires a GPU adapter"]
fn directional_shadows_preserve_indirect_light_geometry_channels_and_alpha_masks()
-> anyhow::Result<()> {
    use gpui_3d::{
        AlphaMode, DirectionalShadow, Light, Object, PbrMaterial, Projection, PunctualLight, Scene,
        Scene3dChannels,
    };
    use std::sync::Arc;
    let mut renderer = HeadlessRenderer::new()?;
    let hole = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 0])),
    )]));
    for pbr in [false, true] {
        let mut receiver = Material::color(rgb(0x908070));
        if pbr {
            receiver = receiver.pbr(PbrMaterial {
                metallic: 0.2,
                roughness: 0.7,
                emissive: [0.03, 0.01, 0.02],
            });
        }
        for softness in [0., 1.5] {
            let mut outputs = Vec::new();
            for case in 0..9 {
                let caster_material = if case == 6 || case == 7 {
                    Material::image(hole.clone()).alpha_mode(if case == 6 {
                        AlphaMode::Mask
                    } else {
                        AlphaMode::Opaque
                    })
                } else {
                    Material::color(rgb(0xffffff))
                        .unlit(true)
                        .alpha_mode(if case == 5 {
                            AlphaMode::Blend
                        } else {
                            AlphaMode::Opaque
                        })
                };
                let scene = Scene::new()
                    .camera(Camera {
                        projection: Projection::Orthographic { vertical_size: 2. },
                        ..Default::default()
                    })
                    .light(Light {
                        ambient: 0.12,
                        ..Default::default()
                    })
                    .lights([
                        PunctualLight::directional([1., 0., 1.]).intensity(0.2),
                        PunctualLight::directional([1., 0., 1.]).intensity(if case == 1 {
                            0.
                        } else {
                            0.8
                        }),
                    ])
                    .directional_shadow((case >= 2).then_some(DirectionalShadow {
                        light_index: 1,
                        resolution: 512,
                        softness,
                        ..DirectionalShadow::new([0.; 3], [3.; 3])
                    }))
                    .object(
                        Object::new(Mesh::plane(), receiver.clone())
                            .scale([4., 4., 1.])
                            .receive_shadows(case != 4),
                    )
                    .object(
                        Object::new(Mesh::plane(), caster_material)
                            .position(if case == 8 {
                                [2.5, 0., 1.5]
                            } else {
                                [1.5, 0., 1.5]
                            })
                            .scale([0.5, 0.5, 1.])
                            .cast_shadows(case != 3),
                    );
                let frame = renderer.render(
                    &scene,
                    Scene3dOutputConfig {
                        color_samples: 1,
                        channels: Scene3dChannels::all(),
                        ..Scene3dOutputConfig::new([65, 65])
                    },
                )?;
                let mut read = frame.readback()?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    if let Some(result) = read.try_read()? {
                        outputs.push(result.pixels);
                        break;
                    }
                    anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            let pixel = 32 * 65 + 32;
            let color =
                |case: usize| &outputs[case].rgba.as_ref().unwrap()[pixel * 4..pixel * 4 + 4];
            assert!(color(0)[0] > color(1)[0] + 15);
            for case in 2..9 {
                let reference = if case == 2 || case == 7 { 1 } else { 0 };
                for (actual, expected) in color(case).iter().zip(color(reference)) {
                    assert!(
                        actual.abs_diff(*expected) <= 2,
                        "case {case}, PBR {pbr}, softness {softness}"
                    );
                }
                assert_eq!(outputs[0].object_ids, outputs[case].object_ids);
                assert_eq!(outputs[0].linear_depth, outputs[case].linear_depth);
                assert_eq!(outputs[0].world_normals, outputs[case].world_normals);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn punctual_sources_match_directional_energy_at_the_surface_center() -> anyhow::Result<()> {
    use gpui_3d::{Light, Object, PbrMaterial, Projection, PunctualLight, Scene};
    let mut renderer = HeadlessRenderer::new()?;
    let point = PunctualLight::point([0., 0., 2.]).intensity(1.6);
    let cases = [
        (vec![], 0.),
        (
            vec![PunctualLight::directional([0., 0., 7.]).intensity(0.4)],
            0.4,
        ),
        (vec![point], 0.4),
        (vec![PunctualLight::point([0., 0., 4.]).intensity(1.6)], 0.1),
        (vec![point.range(Some(2.))], 0.),
        (vec![point.range(Some(4.))], 0.3515625),
        (vec![PunctualLight::point([0.; 3])], 0.),
        (
            vec![
                PunctualLight::spot([0., 0., 2.], [(1_f32 - 0.85 * 0.85).sqrt(), 0., -0.85])
                    .cone_angles(0.95_f32.acos(), 0.75_f32.acos())
                    .intensity(1.6),
            ],
            0.1,
        ),
        (
            vec![
                PunctualLight::point([0., 0., 0.0001])
                    .minimum_distance(0.5)
                    .intensity(0.1),
            ],
            0.4,
        ),
        (
            vec![PunctualLight::spot([0., 0., 2.], [0., 0., -1.]).intensity(1.6)],
            0.4,
        ),
        (
            vec![PunctualLight::spot([0., 0., 2.], [1., 0., 0.]).intensity(1.6)],
            0.,
        ),
        (
            vec![PunctualLight::directional([0., 0., 1.]).intensity(0.05); 8],
            0.4,
        ),
    ];
    for pbr in [false, true] {
        for (lights, expected) in &cases {
            let mut outputs = Vec::new();
            for explicit in [false, true] {
                let material = Material::color(rgb(0x806040));
                let material = if pbr {
                    material.pbr(PbrMaterial {
                        metallic: 0.2,
                        roughness: 0.6,
                        ..Default::default()
                    })
                } else {
                    material
                };
                let mut scene = Scene::new()
                    .camera(Camera {
                        projection: Projection::Orthographic { vertical_size: 2. },
                        ..Default::default()
                    })
                    .light(Light {
                        direction: [0., 0., 1.],
                        intensity: *expected,
                        color: rgb(0xffffff),
                        ambient: 0.,
                    })
                    .object(Object::new(Mesh::plane(), material));
                if explicit {
                    scene = scene.lights(lights.iter().copied());
                }
                let frame = renderer.render(
                    &scene,
                    Scene3dOutputConfig {
                        color_samples: 1,
                        ..Scene3dOutputConfig::new([33, 33])
                    },
                )?;
                let mut read = frame.readback()?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                loop {
                    if let Some(result) = read.try_read()? {
                        outputs.push(result.pixels);
                        break;
                    }
                    anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            }
            assert_eq!(outputs[0].object_ids, outputs[1].object_ids);
            let offset = (16 * 33 + 16) * 4;
            let reference = &outputs[0].rgba.as_ref().unwrap()[offset..offset + 4];
            let actual = &outputs[1].rgba.as_ref().unwrap()[offset..offset + 4];
            assert_eq!(actual[3], 255);
            for (actual, reference) in actual.iter().zip(reference) {
                assert!(
                    actual.abs_diff(*reference) <= 2,
                    "PBR {pbr}, lights {lights:?}: {actual} != {reference}"
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn occlusion_attenuates_only_indirect_light_with_independent_linear_sampling() -> anyhow::Result<()>
{
    use gpui_3d::{
        DiffuseEnvironment, Light, MaterialTexture, Object, PbrMaterial, Projection, Scene,
        Scene3dChannels, TextureAddressMode, TextureSampling, UvTransform,
    };
    use std::sync::Arc;
    let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_fn(2, 1, |x, _| {
            image::Rgba(if x == 0 {
                [0, 255, 255, 255]
            } else {
                [128, 17, 243, 0]
            })
        }),
    )]));
    let map = MaterialTexture::new(image).sampling(TextureSampling {
        transform: UvTransform::from_rows([[0., 0., 1.75], [0., 0., 0.5]])?,
        address_u: TextureAddressMode::Repeat,
        ..Default::default()
    });
    let environment = DiffuseEnvironment::from_equirectangular([1, 1], &[[0.4, 0.8, 0.2]])?;
    let mut renderer = HeadlessRenderer::new()?;
    for (pbr, unlit, strength) in [
        (false, false, 1.),
        (true, false, 0.65),
        (true, false, 0.),
        (true, true, 1.),
    ] {
        let visibility = 1. + strength * (128. / 255. - 1.);
        let mut outputs = Vec::new();
        for mapped in [false, true] {
            let mut material = Material::color(rgb(0x806040));
            if pbr {
                material = material.pbr(PbrMaterial {
                    metallic: 0.25,
                    roughness: 0.6,
                    emissive: [0.05, 0.01, 0.03],
                });
            }
            if mapped {
                material = material
                    .occlusion_texture(map.clone())
                    .occlusion_strength(strength);
            }
            let indirect = if mapped { 1. } else { visibility };
            let scene = Scene::new()
                .camera(Camera {
                    projection: Projection::Orthographic { vertical_size: 2. },
                    ..Default::default()
                })
                .light(Light {
                    direction: [0.3, 0.4, 1.],
                    intensity: 1.,
                    ambient: 0.3 * indirect,
                    ..Default::default()
                })
                .diffuse_environment(environment.intensity(indirect))
                .object(Object::new(Mesh::plane(), material.unlit(unlit)).id("surface"));
            let frame = renderer.render(
                &scene,
                Scene3dOutputConfig {
                    channels: Scene3dChannels::COLOR
                        | Scene3dChannels::OBJECT_ID
                        | Scene3dChannels::LINEAR_DEPTH
                        | Scene3dChannels::WORLD_NORMAL,
                    ..Scene3dOutputConfig::new([33, 33])
                },
            )?;
            let mut read = frame.readback()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                if let Some(result) = read.try_read()? {
                    outputs.push(result.pixels);
                    break;
                }
                anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        assert_eq!(outputs[0].object_ids, outputs[1].object_ids);
        assert_eq!(outputs[0].linear_depth, outputs[1].linear_depth);
        assert_eq!(outputs[0].world_normals, outputs[1].world_normals);
        let expected = outputs[0].rgba.as_ref().unwrap();
        let actual = outputs[1].rgba.as_ref().unwrap();
        assert_eq!(actual[(16 * 33 + 16) * 4 + 3], 255);
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                actual.abs_diff(*expected) <= 2,
                "PBR {pbr}, unlit {unlit}, strength {strength}: {actual} != {expected}"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn geometry_outputs_match_projected_surface_depth_and_vertex_normals() -> anyhow::Result<()> {
    use gpui::{Bounds, RenderImage, point, px, rgba, size};
    use gpui_3d::{
        AlphaMode, MaterialTexture, Object, PbrMaterial, Projection, Scene, Scene3dChannels,
    };
    use std::sync::Arc;
    let normal_map = Arc::new(RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_pixel(1, 1, image::Rgba([230, 180, 220, 255])),
    )]));
    let viewport = Bounds::new(point(px(0.), px(0.)), size(px(67.), px(49.)));
    let mut renderer = HeadlessRenderer::new()?;
    for projection in [
        Projection::default(),
        Projection::Orthographic { vertical_size: 3. },
    ] {
        for side in [-1., 1.] {
            for alpha in [0., 0.25] {
                let camera = Camera {
                    eye: [0.3, 0.2, side * 4.],
                    projection,
                    ..Default::default()
                };
                let mut tint = rgba(0x89c6efff);
                tint.a = alpha;
                let scene = Scene::new()
                    .camera(camera)
                    .object(
                        Object::new(
                            Mesh::cube(),
                            Material::color(tint)
                                .alpha_mode(AlphaMode::Blend)
                                .pbr(PbrMaterial::default())
                                .normal_texture(MaterialTexture::new(normal_map.clone())),
                        )
                        .position([0., 0., side * 0.6])
                        .rotation([0.1, 0.35, 0.])
                        .scale([-1.2, 0.8, 1.]),
                    )
                    .object(
                        Object::new(Mesh::plane(), Material::color(rgb(0xd4a373)))
                            .scale([2.5, 2., 1.]),
                    );
                let frame = renderer.render(
                    &scene,
                    Scene3dOutputConfig {
                        size: [67, 49],
                        channels: Scene3dChannels::OBJECT_ID
                            | Scene3dChannels::LINEAR_DEPTH
                            | Scene3dChannels::WORLD_NORMAL,
                        color_samples: 4,
                    },
                )?;
                assert!(frame.gpu().color().is_none());
                let mut read = frame.readback()?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                let result = loop {
                    if let Some(result) = read.try_read()? {
                        break result;
                    }
                    anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                };
                let pixels = result.pixels;
                let depths = pixels.linear_depth.unwrap();
                let normals = pixels.world_normals.unwrap();
                let ids = pixels.object_ids.unwrap();
                let mut hits = 0;
                for y in (3..49).step_by(7) {
                    for x in (2..67).step_by(7) {
                        let index = y * 67 + x;
                        let p = point(px(x as f32 + 0.5), px(y as f32 + 0.5));
                        if let Some(hit) = scene.pick(viewport, p) {
                            if hit.barycentric.iter().any(|v| *v < 0.005) {
                                continue;
                            }
                            hits += 1;
                            let depth = camera
                                .world_to_screen(viewport, hit.position)?
                                .unwrap()
                                .depth;
                            assert!((depths[index] - depth).abs() < 0.001);
                            assert_eq!(ids[index], hit.object_index as u32 + 1);
                            assert_eq!(normals[index][3], 1.);
                            let orientation = if hit.object_index == 0 { -1. } else { 1. };
                            for (actual, expected) in normals[index][..3].iter().zip(hit.normal) {
                                assert!((actual - expected * orientation).abs() < 0.001);
                            }
                        } else {
                            assert_eq!(ids[index], 0);
                            assert_eq!(depths[index], 0.);
                            assert_eq!(normals[index], [0.; 4]);
                        }
                    }
                }
                assert!(hits > 0);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn normal_maps_match_vertex_normals_across_reflections_and_back_faces() -> anyhow::Result<()> {
    use gpui_3d::{Light, MaterialTexture, Object, PbrMaterial, Projection, Scene};
    use std::sync::Arc;
    let image = Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_pixel(1, 1, image::Rgba([204, 153, 230, 0])),
    )]));
    let material = Material::color(rgb(0x6897ab)).pbr(PbrMaterial {
        metallic: 0.3,
        roughness: 0.6,
        ..Default::default()
    });
    let mut renderer = HeadlessRenderer::new()?;
    for (scale, side) in [
        ([1., 1., 1.], 1.),
        ([-1., 1., 1.], 1.),
        ([1., 1., 1.], -1.),
        ([-1.5, 0.6, 2.], -1.),
    ] {
        let plane = Mesh::plane();
        let mut vertices = plane.vertices().to_vec();
        for vertex in &mut vertices {
            vertex.normal = [
                (204. / 255. * 2. - 1.) * f32::abs(scale[0]),
                -(153. / 255. * 2. - 1.) * f32::abs(scale[1]),
                (230. / 255. * 2. - 1.) * f32::abs(scale[2]),
            ];
        }
        let reference = Mesh::new(vertices, plane.indices().to_vec());
        let mut outputs = Vec::new();
        for (mesh, material) in [
            (reference, material.clone()),
            (
                plane,
                material
                    .clone()
                    .normal_texture(MaterialTexture::new(image.clone())),
            ),
        ] {
            let scene = Scene::new()
                .camera(Camera {
                    eye: [0., 0., side * 3.],
                    projection: Projection::Orthographic { vertical_size: 2. },
                    ..Default::default()
                })
                .light(Light {
                    direction: [-0.4, 0.5, side],
                    color: rgb(0xffffff),
                    intensity: 1.,
                    ambient: 0.1,
                })
                .object(Object::new(mesh, material).scale(scale).id("surface"));
            let frame = renderer.render(&scene, Scene3dOutputConfig::new([65, 65]))?;
            let mut read = frame.readback()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                if let Some(result) = read.try_read()? {
                    outputs.push(result.pixels);
                    break;
                }
                anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
        assert_eq!(outputs[0].object_ids, outputs[1].object_ids);
        let reference = outputs[0].rgba.as_ref().unwrap();
        let mapped = outputs[1].rgba.as_ref().unwrap();
        assert_eq!(mapped[(32 * 65 + 32) * 4 + 3], 255);
        for (actual, expected) in mapped.iter().zip(reference) {
            assert!(
                actual.abs_diff(*expected) <= 2,
                "{actual} != {expected}; scale {scale:?}, side {side}"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn material_maps_match_factors_with_independent_sampling_and_zero_alpha() -> anyhow::Result<()> {
    use gpui_3d::{
        Light, MaterialTexture, Object, PbrMaterial, Projection, Scene, TextureSampling,
        UvTransform,
    };
    use std::sync::Arc;
    let image = |pixels: [[u8; 4]; 2]| {
        Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_fn(2, 1, |x, _| image::Rgba(pixels[x as usize])),
        )]))
    };
    let sampling = |u| TextureSampling {
        transform: UvTransform::from_rows([[0., 0., u], [0., 0., 0.5]]).unwrap(),
        ..Default::default()
    };
    let factors = PbrMaterial {
        metallic: 0.8,
        roughness: 0.9,
        emissive: [0.4, 0.6, 0.8],
    };
    let mapped = Material::color(rgb(0x805030))
        .pbr(factors)
        .metallic_roughness_texture(
            MaterialTexture::new(image([[255; 4], [17, 128, 64, 0]])).sampling(sampling(0.75)),
        )
        .emissive_texture(
            MaterialTexture::new(image([[0; 4], [128, 200, 64, 0]])).sampling(sampling(0.5)),
        );
    let linear = |byte: u8| ((f32::from(byte) / 255. + 0.055) / 1.055).powf(2.4);
    let expected = Material::color(rgb(0x805030)).pbr(PbrMaterial {
        metallic: factors.metallic * 64. / 255.,
        roughness: factors.roughness * 128. / 255.,
        emissive: std::array::from_fn(|i| factors.emissive[i] * 0.5 * linear([128, 200, 64][i])),
    });
    let mut renderer = HeadlessRenderer::new()?;
    let mut outputs = Vec::new();
    for material in [expected, mapped] {
        let scene = Scene::new()
            .camera(Camera {
                projection: Projection::Orthographic { vertical_size: 2. },
                ..Default::default()
            })
            .light(Light {
                direction: [0., 0., 1.],
                color: rgb(0xffffff),
                intensity: 1.,
                ambient: 0.2,
            })
            .object(Object::new(Mesh::plane(), material).id("surface"));
        let frame = renderer.render(&scene, Scene3dOutputConfig::new([65, 65]))?;
        let mut read = frame.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(result) = read.try_read()? {
                outputs.push(result.pixels);
                break;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    assert_eq!(outputs[0].object_ids, outputs[1].object_ids);
    let expected = outputs[0].rgba.as_ref().unwrap();
    let actual = outputs[1].rgba.as_ref().unwrap();
    assert_eq!(actual[(32 * 65 + 32) * 4 + 3], 255);
    for (actual, expected) in actual.iter().zip(expected) {
        assert!(actual.abs_diff(*expected) <= 2, "{actual} != {expected}");
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn pbr_reflection_and_emission_preserve_object_ids() -> anyhow::Result<()> {
    use gpui_3d::{Light, Object, PbrMaterial, Projection, Scene};
    let mut renderer = HeadlessRenderer::new()?;
    let mut reference_ids = None;
    for (metallic, roughness, emissive, unlit, expected) in [
        (0., 0.5, [0.; 3], false, [64_u8; 3]),
        (0., 1., [0.; 3], false, [10; 3]),
        (1., 0.5, [0.; 3], false, [0; 3]),
        (1., 0.5, [0., 0., 0.25], false, [0, 0, 137]),
        (1., 0.5, [0., 0., 0.25], true, [0; 3]),
    ] {
        let scene = Scene::new()
            .camera(Camera {
                projection: Projection::Orthographic { vertical_size: 2. },
                ..Default::default()
            })
            .light(Light {
                direction: [0., 0., 1.],
                color: rgb(0xffffff),
                intensity: 1.,
                ambient: 0.,
            })
            .object(
                Object::new(
                    Mesh::plane(),
                    Material::color(rgb(0))
                        .pbr(PbrMaterial {
                            metallic,
                            roughness,
                            emissive,
                        })
                        .unlit(unlit),
                )
                .id("surface"),
            );
        let frame = renderer.render(&scene, Scene3dOutputConfig::new([65, 65]))?;
        let mut read = frame.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let pixels = loop {
            if let Some(result) = read.try_read()? {
                break result.pixels;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let rgba = pixels.rgba.unwrap();
        let offset = (32 * 65 + 32) * 4;
        for (actual, expected) in rgba[offset..offset + 3].iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 2, "{actual} != {expected}");
        }
        assert_eq!(rgba[offset + 3], 255);
        assert_eq!(&rgba[..4], &[0; 4]);
        if let Some(previous) = &reference_ids {
            assert_eq!(previous, &pixels.object_ids);
        }
        reference_ids = Some(pixels.object_ids);
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn clipped_highlights_preserve_msaa_edges_against_opaque_surfaces() -> anyhow::Result<()> {
    use gpui_3d::{Light, Object, Projection, Scene};
    let mut renderer = HeadlessRenderer::new()?;
    let mut reference = None;
    for ambient in [1., 16.] {
        let scene = Scene::new()
            .camera(Camera {
                projection: Projection::Orthographic { vertical_size: 2. },
                ..Default::default()
            })
            .light(Light {
                ambient,
                intensity: 0.,
                ..Default::default()
            })
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0)))
                    .position([0., 0., -0.1])
                    .scale([4., 4., 1.]),
            )
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0xffffff)))
                    .rotation([0., 0., 0.37])
                    .scale([1.35, 0.85, 1.]),
            );
        let frame = renderer.render(&scene, Scene3dOutputConfig::new([65, 65]))?;
        let mut read = frame.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let pixels = loop {
            if let Some(result) = read.try_read()? {
                break result.pixels;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let rgba = pixels.rgba.as_ref().unwrap();
        assert!(rgba.chunks_exact(4).all(|pixel| pixel[3] == 255));
        assert!(
            rgba.chunks_exact(4)
                .any(|pixel| pixel[0] > 0 && pixel[0] < 255)
        );
        if let Some((color, ids)) = &reference {
            assert_eq!(rgba, color);
            assert_eq!(&pixels.object_ids, ids);
        }
        reference = Some((pixels.rgba.unwrap(), pixels.object_ids));
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn linear_shading_and_display_mapping_preserve_coverage_and_ids() -> anyhow::Result<()> {
    use gpui_3d::{
        ColorOutput, Light, Object, Scene, TextureColorSpace, TextureSampling, ToneMapping,
        UvTransform,
    };
    let mut renderer = HeadlessRenderer::new()?;
    let image = |bytes| {
        std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::from_raw(2, 1, bytes).unwrap(),
        )]))
    };
    let gray = image(vec![128, 128, 128, 255, 128, 128, 128, 255]);
    let edges = image(vec![0, 0, 0, 255, 255, 255, 255, 255]);
    let midpoint = TextureSampling {
        transform: UvTransform::from_rows([[0., 0., 0.5], [0., 0., 0.5]])?,
        ..Default::default()
    };
    let white = Material::color(rgb(0xffffff));
    let mut ids = None;
    let mut coverage = None;
    for (material, ambient, exposure, tone_mapping, expected) in [
        (white.clone(), 0.25, 0., ToneMapping::None, 137_u8),
        (white.clone(), 4., -2., ToneMapping::None, 255),
        (white.clone(), 4., 0., ToneMapping::Reinhard, 231),
        (white, 4., -2., ToneMapping::Reinhard, 188),
        (
            Material::image(gray.clone()).unlit(true),
            0.,
            0.,
            ToneMapping::None,
            128,
        ),
        (
            Material::image(gray)
                .unlit(true)
                .image_color_space(TextureColorSpace::Linear),
            0.,
            0.,
            ToneMapping::None,
            188,
        ),
        (
            Material::image(edges).unlit(true).image_sampling(midpoint),
            0.,
            0.,
            ToneMapping::None,
            188,
        ),
    ] {
        let scene = Scene::new()
            .light(Light {
                ambient,
                intensity: 0.,
                ..Default::default()
            })
            .color_output(ColorOutput {
                exposure,
                tone_mapping,
            })
            .object(Object::new(Mesh::plane(), material).id("surface"));
        let frame = renderer.render(&scene, Scene3dOutputConfig::new([65, 65]))?;
        let mut read = frame.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let pixels = loop {
            if let Some(pixels) = read.try_read()? {
                break pixels.pixels;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        let rgba = pixels.rgba.unwrap();
        let center = (32 * 65 + 32) * 4;
        for value in &rgba[center..center + 3] {
            assert!(value.abs_diff(expected) <= 1, "{value} != {expected}");
        }
        assert_eq!(rgba[center + 3], 255);
        assert_eq!(&rgba[..4], &[0; 4]);
        for pixel in rgba.chunks_exact(4) {
            for channel in &pixel[..3] {
                let expected = f32::from(expected) * f32::from(pixel[3]) / 255.;
                assert!((f32::from(*channel) - expected).abs() <= 2.);
            }
        }
        let alpha = rgba
            .chunks_exact(4)
            .map(|pixel| pixel[3])
            .collect::<Vec<_>>();
        if let Some(previous) = &ids {
            assert_eq!(previous, &pixels.object_ids);
        }
        if let Some(previous) = &coverage {
            assert_eq!(previous, &alpha);
        }
        ids = Some(pixels.object_ids);
        coverage = Some(alpha);
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn image_sampler_controls_color_and_id_cutouts() -> anyhow::Result<()> {
    use gpui_3d::{
        Object, Scene, TextureAddressMode as Address, TextureFilter as Filter, TextureSampling,
        UvTransform,
    };
    let image = std::sync::Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_raw(2, 1, vec![255, 255, 255, 0, 255, 255, 255, 255]).unwrap(),
    )]));
    let mut renderer = HeadlessRenderer::new()?;
    for (address_u, filter, u, cutoff, visible) in [
        (Address::Clamp, Filter::Linear, 0., 0.4, false),
        (Address::Repeat, Filter::Linear, 0., 0.4, true),
        (Address::Repeat, Filter::Nearest, -0.25, 0.9, true),
        (Address::Mirror, Filter::Nearest, 1.25, 0.9, true),
        (Address::Clamp, Filter::Linear, 0.625, 0.9, false),
        (Address::Clamp, Filter::Nearest, 0.625, 0.9, true),
    ] {
        let scene = Scene::new().object(
            Object::new(
                Mesh::plane(),
                Material::image(image.clone())
                    .unlit(true)
                    .alpha_cutoff(cutoff)
                    .image_sampling(TextureSampling {
                        transform: UvTransform::from_rows([[0., 0., u], [0., 0., 0.5]])?,
                        address_u,
                        filter,
                        ..Default::default()
                    }),
            )
            .id("surface"),
        );
        let frame = renderer.render(&scene, Scene3dOutputConfig::new([65, 65]))?;
        let mut read = frame.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let pixels = loop {
            if let Some(pixels) = read.try_read()? {
                break pixels;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(pixels.object_at(32, 32).is_some(), visible);
        assert_eq!(
            pixels.pixels.rgba.as_ref().unwrap()[(32 * 65 + 32) * 4 + 3] > 0,
            visible
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn rendered_ids_retain_node_identity_after_graph_edits() -> anyhow::Result<()> {
    let mut graph = SceneGraph::new();
    let node = graph.insert(
        None,
        Node::new()
            .id("panel")
            .mesh(Mesh::plane(), Material::color(rgb(0x80c0e0)).unlit(true)),
    )?;
    let mut renderer = HeadlessRenderer::new()?;
    let old = renderer.render(
        &graph.evaluate()?.scene(Camera::default()),
        Scene3dOutputConfig::new([65, 65]),
    )?;
    assert_eq!(old.object(1).unwrap().node, Some(node));
    graph.remove_subtree(node)?;
    let replacement = graph.insert(
        None,
        Node::new()
            .id("replacement")
            .mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
    )?;
    let new = renderer.render(
        &graph.evaluate()?.scene(Camera::default()),
        Scene3dOutputConfig::new([40, 30]),
    )?;
    assert_eq!(new.object(1).unwrap().node, Some(replacement));
    assert_ne!(node, replacement);
    drop(renderer);
    drop(graph);
    let mut read = old.readback()?;
    drop(old);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let pixels = loop {
        if let Some(pixels) = read.try_read()? {
            break pixels;
        }
        anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert_eq!(pixels.object_at(32, 32).unwrap().node, Some(node));
    assert_eq!(pixels.object_at(32, 32).unwrap().id, Some("panel".into()));
    assert!(pixels.object_at(0, 0).is_none());
    assert!(pixels.object_at(65, 32).is_none());
    assert!(pixels.object(u32::MAX).is_none());
    Ok(())
}
