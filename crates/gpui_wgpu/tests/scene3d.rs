use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Mesh3d, MeshDraw3d, MeshTexture3d,
    MeshVertex3d, Primitive, Quad, ScaledPixels, Scene, Scene3dFrame, SubtreeLayer, point, rgba,
    size,
};
use gpui_wgpu::WgpuOffscreenRenderer;
use std::{rc::Rc, sync::Arc};

const IDENTITY: [[f32; 4]; 4] = [
    [1., 0., 0., 0.],
    [0., 1., 0., 0.],
    [0., 0., 1., 0.],
    [0., 0., 0., 1.],
];
fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(w), ScaledPixels(h)),
    )
}
fn quad(region: Bounds<ScaledPixels>, color: u32) -> Quad {
    Quad {
        bounds: region,
        content_mask: ContentMask { bounds: region },
        background: rgba(color).into(),
        ..Default::default()
    }
}
fn mesh(z: f32, color: u32, texture: MeshTexture3d) -> MeshDraw3d {
    let vertices = [
        (-0.7, -0.7, [0., 1.]),
        (0.7, -0.7, [1., 1.]),
        (0.7, 0.7, [1., 0.]),
        (-0.7, 0.7, [0., 0.]),
    ]
    .map(|(x, y, uv)| MeshVertex3d {
        position: [x, y, z],
        normal: [0., 0., 1.],
        uv,
    });
    MeshDraw3d {
        cast_shadows: true,
        receive_shadows: true,
        output_id: 1,
        mesh: Mesh3d::new(vertices.to_vec(), vec![0, 1, 2, 0, 2, 3]),
        model: IDENTITY,
        normal: IDENTITY,
        color: rgba(color),
        texture,
        sampling: Default::default(),
        image_color_space: Default::default(),
        pbr: None,
        metallic_roughness_texture: None,
        emissive_texture: None,
        normal_texture: None,
        normal_scale: 1.,
        occlusion_texture: None,
        occlusion_strength: 1.,
        unlit: true,
        alpha_cutoff: 0.5,
        alpha_mode: gpui::AlphaMode3d::Mask,
        sort_depth: 0.,
    }
}
fn layer(
    region: Bounds<ScaledPixels>,
    mut source: Scene,
    objects: Vec<MeshDraw3d>,
    opacity: f32,
) -> SubtreeLayer {
    source.finish();
    SubtreeLayer {
        scene3d: Some(Arc::new(Scene3dFrame {
            viewport_quality: Default::default(),
            background: None,
            specular_environment: None,
            directional_shadow: None,
            ui_texture: None,
            view_projection: IDENTITY,
            world_to_view: [IDENTITY[0], IDENTITY[1], IDENTITY[2], [0., 0., -3., 1.]],
            camera_position: [0., 0., 3.],
            orthographic_view_direction: None,
            light_direction: [0., 0., 1.],
            light: [1.; 4],
            lights: None,
            ambient: 0.3,
            diffuse_environment: None,
            color_output: Default::default(),
            objects: objects.into(),
        })),
        scene: Rc::new(source),
        second_scene: None,
        intermediate_effects: Arc::default(),
        composite: EffectQuad {
            order: 0,
            bounds: region,
            effect_bounds: region,
            transformation: Default::default(),
            content_mask: ContentMask { bounds: region },
            shader: EffectShader::wgsl_image(
                "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }",
            ),
            uniforms: Default::default(),
            time: 0.,
            corner_radii: Default::default(),
            opacity,
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        },
    }
}
fn scene(layer: SubtreeLayer) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(Primitive::SubtreeLayer(layer));
    scene.finish();
    scene
}

#[test]
#[ignore = "requires a GPU adapter"]
fn cache_release_preserves_public_atlas_tiles() -> anyhow::Result<()> {
    use gpui::{PlatformAtlas, TextureMipFilter3d};
    use gpui_wgpu::{Scene3dChannels, Scene3dOutputConfig, WgpuScene3dRenderer};

    let mut renderer = WgpuScene3dRenderer::new_headless()?;
    let atlas = renderer.sprite_atlas().clone();
    let key = gpui::RenderImageParams {
        image_id: gpui::ImageId(99002),
        frame_index: 0,
    }
    .into();
    let tile = atlas
        .get_or_insert_with(&key, &mut || {
            Ok(Some((
                size(DevicePixels(5), DevicePixels(3)),
                std::borrow::Cow::Owned([128, 128, 128, 255].repeat(15)),
            )))
        })?
        .unwrap();
    let mut object = mesh(0.2, 0xffffffff, MeshTexture3d::Image(tile));
    object.sampling.mip_filter = TextureMipFilter3d::Linear;
    let input = layer(bounds(0., 0., 32., 32.), Scene::default(), vec![object], 1.)
        .scene3d
        .unwrap();
    let config = Scene3dOutputConfig {
        size: [32, 32],
        channels: Scene3dChannels::COLOR | Scene3dChannels::OBJECT_ID,
        color_samples: 1,
    };
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let output = renderer.render(&input, config)?;
        renderer.clear_caches();
        assert!(
            atlas
                .get_or_insert_with(&key, &mut || {
                    anyhow::bail!("a live atlas tile was evicted")
                })?
                .is_some()
        );
        let mut pending = output.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(pixels) = pending.try_read()? {
                outputs.push(pixels);
                break;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }
    assert_eq!(outputs[0].rgba, outputs[1].rgba);
    assert_eq!(outputs[0].object_ids, outputs[1].object_ids);
    assert_eq!(outputs[1].object_ids.as_ref().unwrap()[16 * 32 + 16], 1);
    let offset = (16 * 32 + 16) * 4;
    let color = &outputs[1].rgba.as_ref().unwrap()[offset..offset + 4];
    assert!(color[..3].iter().all(|&value| value.abs_diff(128) <= 1));
    assert_eq!(color[3], 255);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn image_mips_preserve_linear_energy_crop_and_reallocated_atlas_content() -> anyhow::Result<()> {
    use gpui::{PlatformAtlas, TextureColorSpace3d, TextureMipFilter3d};
    use gpui_wgpu::{Scene3dChannels, Scene3dOutputConfig, WgpuScene3dRenderer};
    let mut renderer = WgpuScene3dRenderer::new_headless()?;
    let atlas = renderer.sprite_atlas().clone();
    let key = gpui::AtlasKey::Image(gpui::RenderImageParams {
        image_id: gpui::ImageId(99000),
        frame_index: 0,
    });
    // A neighboring allocation keeps the backing texture alive during tile reuse.
    atlas.get_or_insert_with(
        &gpui::RenderImageParams {
            image_id: gpui::ImageId(99001),
            frame_index: 0,
        }
        .into(),
        &mut || {
            Ok(Some((
                size(DevicePixels(64), DevicePixels(64)),
                std::borrow::Cow::Owned([255, 0, 0, 255].repeat(64 * 64)),
            )))
        },
    )?;
    for replacement in [false, true] {
        let bytes: Vec<u8> = (0..15)
            .flat_map(|i| {
                if replacement {
                    [255, 0, 255, 255]
                } else {
                    let channel = [0, 64, 128, 192, 255][i % 5];
                    [channel, channel, channel, 255]
                }
            })
            .collect();
        let tile = atlas
            .get_or_insert_with(&key, &mut || {
                Ok(Some((
                    size(DevicePixels(5), DevicePixels(3)),
                    std::borrow::Cow::Borrowed(&bytes),
                )))
            })?
            .unwrap();
        for color_space in [TextureColorSpace3d::Srgb, TextureColorSpace3d::Linear] {
            for (mip_filter, anisotropy) in [
                (TextureMipFilter3d::Nearest, 1),
                (TextureMipFilter3d::Linear, 1),
                (TextureMipFilter3d::Linear, 16),
            ] {
                let mut object = mesh(0.2, 0xffffffff, MeshTexture3d::Image(tile));
                object.alpha_mode = gpui::AlphaMode3d::Opaque;
                object.image_color_space = color_space;
                object.sampling = gpui::TextureSampling3d {
                    transform: gpui::UvTransform3d::from_scale_rotation_translation(
                        [4096.; 2], 0.3, [0.; 2],
                    )?,
                    address_u: gpui::TextureAddressMode3d::Repeat,
                    address_v: gpui::TextureAddressMode3d::Mirror,
                    mip_filter,
                    max_anisotropy: anisotropy,
                    ..Default::default()
                };
                let input = layer(bounds(0., 0., 32., 32.), Scene::default(), vec![object], 1.)
                    .scene3d
                    .unwrap();
                let output = renderer.render(
                    &input,
                    Scene3dOutputConfig {
                        size: [32, 32],
                        channels: Scene3dChannels::LINEAR_COLOR | Scene3dChannels::OBJECT_ID,
                        color_samples: 1,
                    },
                )?;
                let mut read = output.readback()?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
                let pixels = loop {
                    if let Some(pixels) = read.try_read()? {
                        break pixels;
                    }
                    anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                };
                let expected: [f32; 3] = std::array::from_fn(|channel| {
                    bytes
                        .chunks_exact(4)
                        .map(|pixel| {
                            let value = f32::from(pixel[channel]) / 255.;
                            if color_space == TextureColorSpace3d::Linear || value <= 0.04045 {
                                if color_space == TextureColorSpace3d::Linear {
                                    value
                                } else {
                                    value / 12.92
                                }
                            } else {
                                ((value + 0.055) / 1.055).powf(2.4)
                            }
                        })
                        .sum::<f32>()
                        / 15.
                });
                for y in 12..20 {
                    for x in 12..20 {
                        let index = y * 32 + x;
                        assert_eq!(pixels.object_ids.as_ref().unwrap()[index], 1);
                        let actual = pixels.linear_rgba.as_ref().unwrap()[index];
                        for channel in 0..3 {
                            assert!(
                                (actual[channel] - expected[channel]).abs() < 0.002,
                                "{actual:?} != {expected:?}"
                            );
                        }
                        assert_eq!(actual[3], 1.);
                    }
                }
            }
        }
        atlas.remove(&key);
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn instance_updates_preserve_owned_color_and_geometry_outputs() -> anyhow::Result<()> {
    use gpui_wgpu::{Scene3dChannels, Scene3dOutputConfig, WgpuScene3dRenderer};
    let mut renderer = WgpuScene3dRenderer::new_headless()?;
    let mut left = mesh(0.2, 0xff0000ff, MeshTexture3d::None);
    left.model[0][0] = 0.4;
    left.model[1][1] = 0.4;
    left.model[3][0] = -0.5;
    left.output_id = 7;
    let mut right = left.clone();
    right.model[3] = [0.5, 0., 0.2, 1.];
    right.normal[2] = [0.6, 0., 0.8, 0.];
    right.color = rgba(0x00ff00ff);
    right.output_id = 19;
    let mut masked = left.clone();
    masked.model[3][2] = -0.1;
    masked.color.a = 0.;
    masked.output_id = 99;
    let mut grown = vec![left.clone(), right.clone()];
    grown.extend(std::iter::repeat_n(masked, 19));
    let mut outputs = Vec::new();
    for (objects, shaded, ids) in [
        (vec![left.clone(), right.clone()], true, [7, 19]),
        (vec![right.clone()], true, [0, 19]),
        (grown, true, [7, 19]),
        (vec![], true, [0, 0]),
        (vec![left.clone(), right.clone()], false, [7, 19]),
        (vec![left, right], true, [7, 19]),
    ] {
        let input = layer(bounds(0., 0., 64., 64.), Scene::default(), objects, 1.)
            .scene3d
            .unwrap();
        let channels = if shaded {
            Scene3dChannels::all()
        } else {
            Scene3dChannels::OBJECT_ID
                | Scene3dChannels::LINEAR_DEPTH
                | Scene3dChannels::WORLD_NORMAL
        };
        outputs.push((
            renderer.render(
                &input,
                Scene3dOutputConfig {
                    size: [64, 64],
                    channels,
                    color_samples: 1,
                },
            )?,
            ids,
        ));
    }
    for (output, ids) in outputs {
        let mut read = output.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let pixels = loop {
            if let Some(pixels) = read.try_read()? {
                break pixels;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        for (x, id) in [16, 48].into_iter().zip(ids) {
            let index = 32 * 64 + x;
            assert_eq!(pixels.object_ids.as_ref().unwrap()[index], id);
            let (color, depth, normal) = match id {
                7 => ([255, 0, 0, 255], 2.8, [0., 0., 1., 1.]),
                19 => ([0, 255, 0, 255], 2.6, [0.6, 0., 0.8, 1.]),
                _ => ([0; 4], 0., [0.; 4]),
            };
            if let Some(rgba) = &pixels.rgba {
                assert_eq!(&rgba[index * 4..index * 4 + 4], &color);
            }
            assert!((pixels.linear_depth.as_ref().unwrap()[index] - depth).abs() < 1e-5);
            for (actual, expected) in pixels.world_normals.as_ref().unwrap()[index]
                .into_iter()
                .zip(normal)
            {
                assert!((actual - expected).abs() < 1e-5);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn instance_batches_keep_concurrent_viewport_uploads_independent() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(64)))?;
    let mut left = mesh(0.2, 0xff0000ff, MeshTexture3d::None);
    left.model[0][0] = 0.4;
    left.model[1][1] = 0.4;
    left.model[3][0] = -0.5;
    let mut right = left.clone();
    right.model[3][0] = 0.5;
    right.color = rgba(0x0000ffff);
    let mut other = left.clone();
    other.color = rgba(0x00ff00ff);
    let mut input = Scene::default();
    for (x, objects) in [(0., vec![left, right]), (64., vec![other])] {
        input.insert_primitive(Primitive::SubtreeLayer(layer(
            bounds(x, 0., 64., 64.),
            Scene::default(),
            objects,
            1.,
        )));
    }
    input.finish();
    for _ in 0..2 {
        let pixels = renderer.render_rgba(&input)?;
        for (x, color) in [
            (16, [255, 0, 0, 255]),
            (48, [0, 0, 255, 255]),
            (80, [0, 255, 0, 255]),
            (112, [0; 4]),
        ] {
            let index = (32 * 128 + x) * 4;
            assert_eq!(&pixels[index..index + 4], &color);
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn vertex_snapshots_preserve_concurrent_views_and_replayed_frames() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(64)))?;
    let original = mesh(0.2, 0xff0000ff, MeshTexture3d::None);
    let mut updated = original.clone();
    let vertices = original
        .mesh
        .vertices()
        .iter()
        .map(|v| MeshVertex3d {
            position: [v.position[0] + 1.2, v.position[1], 0.4],
            ..*v
        })
        .collect();
    updated.mesh = original.mesh.with_vertices(vertices, None)?;
    updated.color = rgba(0x00ff00ff);
    for objects in [
        vec![original.clone()],
        vec![updated.clone()],
        vec![original.clone()],
    ] {
        renderer.render_rgba(&scene(layer(
            bounds(0., 0., 64., 64.),
            Scene::default(),
            objects,
            1.,
        )))?;
    }
    let mut input = Scene::default();
    input.insert_primitive(Primitive::SubtreeLayer(layer(
        bounds(0., 0., 64., 64.),
        Scene::default(),
        vec![original],
        1.,
    )));
    input.insert_primitive(Primitive::SubtreeLayer(layer(
        bounds(64., 0., 64., 64.),
        Scene::default(),
        vec![updated],
        1.,
    )));
    input.finish();
    let pixels = renderer.render_rgba(&input)?;
    let at = |x: usize| &pixels[(32 * 128 + x) * 4..(32 * 128 + x) * 4 + 4];
    assert_eq!(at(32), &[255, 0, 0, 255]);
    assert_eq!(at(96), &[0; 4]);
    assert_eq!(at(120), &[0, 255, 0, 255]);
    assert_eq!(renderer.render_rgba(&input)?, pixels);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn vertex_updates_keep_owned_outputs_and_geometry_channels_in_sync() -> anyhow::Result<()> {
    use gpui_wgpu::{Scene3dChannels, Scene3dOutputConfig, WgpuScene3dRenderer};
    let mut renderer = WgpuScene3dRenderer::new_headless()?;
    let original = mesh(0.2, 0xff0000ff, MeshTexture3d::None);
    let mut updated = original.clone();
    updated.mesh = original.mesh.with_vertices(
        original
            .mesh
            .vertices()
            .iter()
            .map(|v| MeshVertex3d {
                position: [v.position[0] + 1.2, v.position[1], 0.4],
                normal: [1., 0., 1.],
                ..*v
            })
            .collect(),
        None,
    )?;
    updated.color = rgba(0x00ff00ff);
    updated.output_id = 2;
    let mut outputs = Vec::new();
    for (objects, shaded, center_id, right_id) in [
        (vec![original.clone()], true, 1, 0),
        (vec![updated.clone()], true, 0, 2),
        (vec![original.clone(), updated.clone()], true, 1, 2),
        (vec![original.clone()], false, 1, 0),
        (vec![updated.clone()], false, 0, 2),
        (vec![original, updated], true, 1, 2),
    ] {
        let input = layer(bounds(0., 0., 64., 64.), Scene::default(), objects, 1.)
            .scene3d
            .unwrap();
        let channels = if shaded {
            Scene3dChannels::all()
        } else {
            Scene3dChannels::OBJECT_ID
                | Scene3dChannels::LINEAR_DEPTH
                | Scene3dChannels::WORLD_NORMAL
        };
        outputs.push((
            renderer.render(
                &input,
                Scene3dOutputConfig {
                    size: [64, 64],
                    channels,
                    color_samples: 1,
                },
            )?,
            center_id,
            right_id,
        ));
    }
    for (output, center_id, right_id) in outputs {
        let mut read = output.readback()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let pixels = loop {
            if let Some(pixels) = read.try_read()? {
                break pixels;
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        for (index, id) in [(32 * 64 + 32, center_id), (32 * 64 + 56, right_id)] {
            assert_eq!(pixels.object_ids.as_ref().unwrap()[index], id);
            let depth = pixels.linear_depth.as_ref().unwrap()[index];
            let normal = pixels.world_normals.as_ref().unwrap()[index];
            let expected = match id {
                1 => ([255, 0, 0, 255], 2.8, [0., 0., 1., 1.]),
                2 => (
                    [0, 255, 0, 255],
                    2.6,
                    [
                        std::f32::consts::FRAC_1_SQRT_2,
                        0.,
                        std::f32::consts::FRAC_1_SQRT_2,
                        1.,
                    ],
                ),
                _ => ([0; 4], 0., [0.; 4]),
            };
            if let Some(rgba) = &pixels.rgba {
                assert_eq!(&rgba[index * 4..index * 4 + 4], &expected.0);
            }
            assert!((depth - expected.1).abs() < 1e-5);
            for (a, b) in normal.into_iter().zip(expected.2) {
                assert!((a - b).abs() < 1e-5);
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn environment_backgrounds_preserve_viewport_clipping_opacity_and_shared_maps() -> anyhow::Result<()>
{
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(96)))?;
    let map = gpui::EnvironmentMap3d::from_equirectangular([1, 1], vec![[1., 0., 0.]])?;
    let mut left = layer(bounds(8., 12., 48., 60.), Scene::default(), vec![], 1.);
    Arc::make_mut(left.scene3d.as_mut().unwrap()).background =
        Some(gpui::EnvironmentBackground3d {
            map,
            intensity: 1.,
            rotation_y: 0.,
            rays: [[0., 0., -1.], [1., 0., 0.], [0., 1., 0.]],
        });
    let mut right = left.clone();
    right.composite.bounds = bounds(72., 12., 48., 60.);
    right.composite.effect_bounds = right.composite.bounds;
    right.composite.content_mask.bounds = bounds(72., 12., 24., 60.);
    right.composite.opacity = 0.5;
    let mut input = Scene::default();
    input.insert_primitive(Primitive::SubtreeLayer(left.clone()));
    input.insert_primitive(Primitive::SubtreeLayer(right));
    input.finish();
    let pixels = renderer.render_rgba(&input)?;
    let at = |x: usize, y: usize| &pixels[(y * 128 + x) * 4..(y * 128 + x) * 4 + 4];
    assert_eq!(at(30, 30), &[255, 0, 0, 255]);
    assert!(at(80, 30)[0].abs_diff(128) <= 1);
    assert!(at(80, 30)[3].abs_diff(128) <= 1);
    assert_eq!(at(104, 30), &[0; 4]);
    assert_eq!(at(64, 30), &[0; 4]);
    assert_eq!(at(30, 80), &[0; 4]);
    let original = renderer.render_rgba(&scene(left.clone()))?;
    Arc::make_mut(left.scene3d.as_mut().unwrap()).background = None;
    let cleared = renderer.render_rgba(&scene(left))?;
    assert_eq!(
        &original[(30 * 128 + 30) * 4..(30 * 128 + 30) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert!(cleared.iter().all(|v| *v == 0));
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn blended_layers_preserve_linear_color_depth_and_nearest_ids() -> anyhow::Result<()> {
    use gpui_wgpu::{Scene3dChannels, Scene3dOutputConfig, WgpuScene3dRenderer};
    let mut renderer = WgpuScene3dRenderer::new_headless()?;
    let mut front = mesh(0.2, 0xff000080, MeshTexture3d::None);
    front.alpha_mode = gpui::AlphaMode3d::Blend;
    front.sort_depth = 0.2;
    front.output_id = 7;
    let mut rear = mesh(0.8, 0x0000ff80, MeshTexture3d::None);
    rear.alpha_mode = gpui::AlphaMode3d::Blend;
    rear.sort_depth = 0.8;
    rear.output_id = 9;
    let mut blocker = mesh(0.1, 0x00ff0000, MeshTexture3d::None);
    blocker.alpha_mode = gpui::AlphaMode3d::Opaque;
    blocker.output_id = 11;
    for samples in [1, 4] {
        for case in 0..4 {
            let mut front = front.clone();
            let mut rear = rear.clone();
            let (expected, id) = match case {
                0 => ([160, 0, 117, 192], 7),
                1 => {
                    rear.alpha_mode = gpui::AlphaMode3d::Opaque;
                    ([188, 0, 187, 255], 7)
                }
                2 => {
                    front.color.a = 0.;
                    ([0, 0, 128, 128], 9)
                }
                _ => ([0, 255, 0, 255], 11),
            };
            let mut objects = vec![front, rear];
            if case == 3 {
                objects.push(blocker.clone());
            }
            for reverse in [false, true] {
                if reverse {
                    objects.reverse();
                }
                let input = layer(
                    bounds(0., 0., 32., 32.),
                    Scene::default(),
                    objects.clone(),
                    1.,
                )
                .scene3d
                .unwrap();
                let output = renderer.render(
                    &input,
                    Scene3dOutputConfig {
                        size: [32, 32],
                        channels: Scene3dChannels::COLOR | Scene3dChannels::OBJECT_ID,
                        color_samples: samples,
                    },
                )?;
                let mut read = output.readback()?;
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                let pixels = loop {
                    if let Some(pixels) = read.try_read()? {
                        break pixels;
                    }
                    anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
                    std::thread::sleep(std::time::Duration::from_millis(2));
                };
                let center = 16 * 32 + 16;
                assert_eq!(pixels.object_ids.unwrap()[center], id);
                for (actual, expected) in pixels.rgba.unwrap()[center * 4..center * 4 + 4]
                    .iter()
                    .zip(expected)
                {
                    assert!(
                        (i32::from(*actual) - expected).abs() <= 2,
                        "case {case}, samples {samples}, reverse {reverse}: {actual} != {expected}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn direct_outputs_preserve_integer_ids_cutouts_and_frame_lifetimes() -> anyhow::Result<()> {
    use gpui_wgpu::{Scene3dChannels, Scene3dOutputConfig, WgpuScene3dRenderer};
    let mut renderer = WgpuScene3dRenderer::new_headless()?;
    let tile = gpui::PlatformAtlas::get_or_insert_with(
        renderer.sprite_atlas().as_ref(),
        &gpui::RenderImageParams {
            image_id: gpui::ImageId(98001),
            frame_index: 0,
        }
        .into(),
        &mut || {
            Ok(Some((
                size(DevicePixels(2), DevicePixels(1)),
                std::borrow::Cow::Borrowed(&[255, 255, 255, 255, 255, 255, 255, 0]),
            )))
        },
    )?
    .unwrap();
    let mut near = mesh(0.2, 0xff0000ff, MeshTexture3d::Image(tile));
    near.output_id = 0xfe12_ab34;
    let mut far = mesh(0.8, 0x0000ffff, MeshTexture3d::None);
    far.output_id = 0x1000_0001;
    let mut input = layer(
        bounds(0., 0., 67., 49.),
        Scene::default(),
        vec![near.clone(), far.clone()],
        1.,
    )
    .scene3d
    .unwrap()
    .as_ref()
    .clone();
    let config = Scene3dOutputConfig {
        size: [67, 49],
        channels: Scene3dChannels::COLOR | Scene3dChannels::OBJECT_ID,
        color_samples: 1,
    };
    let first = renderer.render(&input, config)?;
    let planned = gpui_wgpu::Scene3dDrawStatistics::plan(
        &input,
        config.channels,
        renderer.max_instances_per_batch(),
    )?;
    assert_eq!(first.draw_statistics(), planned);
    assert_eq!(planned.camera_draws, 4);
    assert_eq!(planned.camera_instances, 4);
    input.objects = vec![far, near].into();
    let reversed = renderer.render(&input, config)?;
    input.objects = Arc::default();
    let empty = renderer.render(
        &input,
        Scene3dOutputConfig {
            size: [31, 17],
            ..config
        },
    )?;
    assert_eq!(empty.draw_statistics(), Default::default());
    assert_eq!(first.draw_statistics(), planned);
    assert_eq!(reversed.draw_statistics(), planned);

    let mut read = first.readback()?;
    assert!(reversed.readback().is_err());
    let poll = |read: &mut gpui_wgpu::Scene3dReadback| -> anyhow::Result<gpui_wgpu::Scene3dPixels> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if let Some(pixels) = read.try_read()? {
                return Ok(pixels);
            }
            anyhow::ensure!(std::time::Instant::now() < deadline, "readback timed out");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    };
    let first_pixels = poll(&mut read)?;
    assert!(read.try_read().is_err());
    let ids = first_pixels.object_ids.as_ref().unwrap();
    let rgba = first_pixels.rgba.as_ref().unwrap();
    assert_eq!(ids.len(), 67 * 49);
    assert_eq!(rgba.len(), 67 * 49 * 4);
    assert_eq!(ids[24 * 67 + 20], 0xfe12_ab34);
    assert_eq!(ids[24 * 67 + 47], 0x1000_0001);
    assert_eq!(
        &rgba[(24 * 67 + 20) * 4..(24 * 67 + 20) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(
        &rgba[(24 * 67 + 47) * 4..(24 * 67 + 47) * 4 + 4],
        &[0, 0, 255, 255]
    );
    assert_eq!(ids[0], 0);
    assert_eq!(&rgba[..4], &[0; 4]);
    let reversed_pixels = poll(&mut reversed.readback()?)?;
    assert_eq!(first_pixels.object_ids, reversed_pixels.object_ids);
    assert_eq!(first_pixels.rgba, reversed_pixels.rgba);
    let empty_pixels = poll(&mut empty.readback()?)?;
    assert_eq!(empty_pixels.size, [31, 17]);
    assert!(empty_pixels.rgba.unwrap().iter().all(|v| *v == 0));
    assert!(empty_pixels.object_ids.unwrap().iter().all(|v| *v == 0));

    let ids_only = renderer.render(
        &input,
        Scene3dOutputConfig {
            channels: Scene3dChannels::OBJECT_ID,
            ..config
        },
    )?;
    assert!(ids_only.color().is_none());
    let pending = ids_only.readback()?;
    drop(pending);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut next = loop {
        if let Ok(read) = ids_only.readback() {
            break read;
        }
        anyhow::ensure!(
            std::time::Instant::now() < deadline,
            "cancelled readback retained its permit"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    };
    assert!(poll(&mut next)?.rgba.is_none());
    assert!(
        renderer
            .render(&input, Scene3dOutputConfig::new([0, 10]))
            .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn viewport_quality_keeps_mixed_samples_and_ui_texture_coordinates_independent()
-> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(208), DevicePixels(100)))?;
    for scale in [0.5, 1., 2., 0.5] {
        for samples in [1, 4] {
            let mut source = Scene::default();
            source.insert_primitive(quad(bounds(8.25, 12.75, 40.25, 64.5), 0x00ff00ff));
            source.insert_primitive(quad(bounds(48.5, 12.75, 40.25, 64.5), 0x0000ffff));
            let mut left = layer(
                bounds(8.25, 12.75, 80.5, 64.5),
                source,
                vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
                1.,
            );
            Arc::make_mut(left.scene3d.as_mut().unwrap()).viewport_quality =
                gpui::Scene3dViewportQuality::new(scale, samples);
            let mut right = layer(
                bounds(108.25, 12.75, 80.5, 64.5),
                Scene::default(),
                vec![mesh(0.2, 0xff0000ff, MeshTexture3d::None)],
                1.,
            );
            Arc::make_mut(right.scene3d.as_mut().unwrap()).viewport_quality =
                gpui::Scene3dViewportQuality::new(1. / scale, if samples == 1 { 4 } else { 1 });
            let mut input = Scene::default();
            input.insert_primitive(Primitive::SubtreeLayer(left));
            input.insert_primitive(Primitive::SubtreeLayer(right));
            input.finish();
            let pixels = renderer.render_rgba(&input)?;
            for (x, y, expected) in [
                (30, 45, [0, 255, 0, 255]),
                (70, 45, [0, 0, 255, 255]),
                (150, 45, [255, 0, 0, 255]),
                (100, 45, [0; 4]),
                (150, 90, [0; 4]),
            ] {
                let index = (y * 208 + x) * 4;
                assert_eq!(
                    &pixels[index..index + 4],
                    &expected,
                    "scale {scale}, samples {samples}, pixel {x}, {y}"
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn viewport_pixel_mapping_survives_offset_clipping_and_target_resize() -> anyhow::Result<()> {
    let make = |dx: f32, dy: f32| {
        let region = bounds(8.25 + dx, 8.75 + dy, 64.5, 48.5);
        let mut source = Scene::default();
        source.insert_primitive(quad(bounds(8.25 + dx, 8.75 + dy, 32.25, 48.5), 0x00ff00ff));
        source.insert_primitive(quad(bounds(40.5 + dx, 8.75 + dy, 32.25, 48.5), 0x0000ffff));
        scene(layer(
            region,
            source,
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
            1.,
        ))
    };
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(96), DevicePixels(80)))?;
    let reference = renderer.render_rgba(&make(0., 0.))?;
    assert_eq!(
        &reference[(32 * 96 + 24) * 4..(32 * 96 + 24) * 4 + 4],
        &[0, 255, 0, 255]
    );
    assert_eq!(
        &reference[(32 * 96 + 56) * 4..(32 * 96 + 56) * 4 + 4],
        &[0, 0, 255, 255]
    );
    for (width, height, dx, dy) in [
        (256, 192, 37, 21),
        (96, 80, -24, -16),
        (96, 80, 48, 40),
        (96, 80, 0, 0),
    ] {
        renderer.resize(size(DevicePixels(width), DevicePixels(height)));
        let pixels = renderer.render_rgba(&make(dx as f32, dy as f32))?;
        for y in 0..height {
            for x in 0..width {
                let (rx, ry) = (x - dx, y - dy);
                let expected = if (0..96).contains(&rx) && (0..80).contains(&ry) {
                    let index = ((ry * 96 + rx) * 4) as usize;
                    &reference[index..index + 4]
                } else {
                    &[0; 4]
                };
                let index = ((y * width + x) * 4) as usize;
                for (actual, expected) in pixels[index..index + 4].iter().zip(expected) {
                    assert!(
                        actual.abs_diff(*expected) <= 1,
                        "pixel {x}, {y}: {actual} != {expected}"
                    );
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn mesh_depth_capture_clipping_and_nested_composition() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(96)))?;
    let tile = renderer
        .sprite_atlas()
        .get_or_insert_with(
            &gpui::RenderImageParams {
                image_id: gpui::ImageId(98000),
                frame_index: 0,
            }
            .into(),
            &mut || {
                Ok(Some((
                    size(DevicePixels(1), DevicePixels(1)),
                    std::borrow::Cow::Borrowed(&[255, 0, 255, 255]),
                )))
            },
        )?
        .expect("image tile");
    for scale in [1., 1.5, 2.] {
        let width = (128. * scale) as usize;
        renderer.resize(size(
            DevicePixels(width as i32),
            DevicePixels((96. * scale) as i32),
        ));
        let region = bounds(8. * scale, 8. * scale, 112. * scale, 80. * scale);
        let pixel = |x: usize, y: usize| {
            (((y as f32 * scale) as usize) * width + (x as f32 * scale) as usize) * 4
        };
        let near = mesh(0.2, 0xff0000ff, MeshTexture3d::None);
        let textured = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Image(tile))],
            1.,
        )))?;
        assert_eq!(&textured[pixel(64, 48)..pixel(64, 48) + 3], &[255, 0, 255]);
        let far = mesh(0.8, 0x0000ffff, MeshTexture3d::None);
        let a = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![near.clone(), far.clone()],
            1.,
        )))?;
        let b = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![far.clone(), near.clone()],
            1.,
        )))?;
        assert_eq!(a, b);
        assert_eq!(&a[pixel(64, 48)..pixel(64, 48) + 3], &[255, 0, 0]);
        let mut source = Scene::default();
        source.insert_primitive(quad(
            bounds(8. * scale, 8. * scale, 56. * scale, 80. * scale),
            0x00ff00ff,
        ));
        let captured = layer(
            region,
            source,
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree), far.clone()],
            1.,
        );
        let output = renderer.render_rgba(&scene(captured.clone()))?;
        assert_eq!(&output[pixel(40, 48)..pixel(40, 48) + 3], &[0, 255, 0]);
        assert_eq!(&output[pixel(90, 48)..pixel(90, 48) + 3], &[0, 0, 255]);
        let mut clipped = captured.clone();
        clipped.composite.content_mask.bounds = bounds(0., 0., 64. * scale, 96. * scale);
        let output = renderer.render_rgba(&scene(clipped))?;
        assert_eq!(&output[pixel(90, 48)..pixel(90, 48) + 3], &[0, 0, 0]);
        let mut nested = layer(
            region,
            scene(captured),
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
            0.5,
        );
        let outer = scene(nested.clone());
        assert_eq!(outer.subtree_target_count(), 3);
        let faded = renderer.render_rgba(&outer)?;
        nested.composite.opacity = 1.;
        let bright = renderer.render_rgba(&scene(nested))?;
        assert!(
            faded[pixel(40, 48) + 1] > 0 && faded[pixel(40, 48) + 1] < bright[pixel(40, 48) + 1]
        );
        let clipped_depth = renderer.render_rgba(&scene(layer(
            region,
            Scene::default(),
            vec![mesh(-0.1, 0xff0000ff, MeshTexture3d::None), far],
            1.,
        )))?;
        assert_eq!(
            &clipped_depth[pixel(64, 48)..pixel(64, 48) + 3],
            &[0, 0, 255]
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn independent_ui_textures_resize_and_compose_without_source_clipping() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(128), DevicePixels(96)))?;
    let region = bounds(8., 8., 112., 80.);
    let sample =
        |image: &[u8], x: usize, y: usize| image[(y * 128 + x) * 4..(y * 128 + x) * 4 + 3].to_vec();
    for density in [0.5, 2., 1., 8.] {
        let config = gpui::UiTexture3d::new(size(gpui::px(640.), gpui::px(400.)), density);
        let physical = config.pixel_size();
        let w = physical.width.0 as f32;
        let h = physical.height.0 as f32;
        let mut source = Scene::default();
        source.insert_primitive(quad(bounds(0., 0., w * 0.7, h), 0x00ff00ff));
        let mut capture = layer(
            region,
            source,
            vec![
                mesh(0.2, 0xffffffff, MeshTexture3d::Subtree),
                mesh(0.8, 0x0000ffff, MeshTexture3d::None),
            ],
            1.,
        );
        Arc::make_mut(capture.scene3d.as_mut().unwrap()).ui_texture = Some(config);
        let output = renderer.render_rgba(&scene(capture.clone()))?;
        assert_eq!(sample(&output, 40, 48), [0, 255, 0]);
        assert_eq!(sample(&output, 70, 48), [0, 255, 0]);
        assert_eq!(sample(&output, 96, 48), [0, 0, 255]);
        assert_eq!(sample(&output, 2, 48), [0, 0, 0]);

        capture.composite.content_mask.bounds = bounds(0., 0., 64., 96.);
        let output = renderer.render_rgba(&scene(capture.clone()))?;
        assert_eq!(sample(&output, 40, 48), [0, 255, 0]);
        assert_eq!(sample(&output, 70, 48), [0, 0, 0]);

        capture.composite.content_mask.bounds = region;
        let mut nested = layer(
            region,
            scene(capture),
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
            0.5,
        );
        Arc::make_mut(nested.scene3d.as_mut().unwrap()).ui_texture = Some(gpui::UiTexture3d::new(
            size(gpui::px(128.), gpui::px(96.)),
            1.,
        ));
        let output = renderer.render_rgba(&scene(nested))?;
        let green = sample(&output, 48, 48)[1];
        assert!(green > 80 && green < 220);
    }

    let mut root = Scene::default();
    for (x, color, width) in [(0., 0xff0000ff, 320.), (64., 0x00ff0080, 640.)] {
        let config = gpui::UiTexture3d::new(size(gpui::px(width), gpui::px(240.)), 2.);
        let physical = config.pixel_size();
        let mut source = Scene::default();
        source.insert_primitive(quad(
            bounds(0., 0., physical.width.0 as f32, physical.height.0 as f32),
            color,
        ));
        let mut capture = layer(
            bounds(x, 0., 64., 96.),
            source,
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
            1.,
        );
        Arc::make_mut(capture.scene3d.as_mut().unwrap()).ui_texture = Some(config);
        root.insert_primitive(Primitive::SubtreeLayer(capture));
    }
    root.finish();
    let output = renderer.render_rgba(&root)?;
    assert_eq!(sample(&output, 32, 48), [255, 0, 0]);
    assert_eq!(sample(&output, 96, 48), [0, 255, 0]);
    renderer.resize(size(DevicePixels(256), DevicePixels(192)));
    let resized = renderer.render_rgba(&root)?;
    for x in [32, 96] {
        assert_eq!(
            &resized[(48 * 256 + x) * 4..(48 * 256 + x) * 4 + 3],
            sample(&output, x, 48)
        );
    }
    Ok(())
}
