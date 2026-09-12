use crate::metal_renderer::{InstanceBufferPool, MetalRenderer};
use gpui::{
    Bounds, ContentMask, DevicePixels, EffectQuad, EffectShader, Mesh3d, MeshDraw3d, MeshTexture3d,
    MeshVertex3d, Primitive, Quad, ScaledPixels, Scene, Scene3dFrame, SubtreeLayer, point, rgba,
    size,
};
use parking_lot::Mutex;
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
        mesh_passes: Default::default(),
        custom_material: None,
        gpu_geometry: None,
        render_bounds: None,
        cast_shadows: true,
        receive_shadows: true,
        output_id: 1,
        mesh: Mesh3d::new(vertices.to_vec(), vec![0, 1, 2, 0, 2, 3]),
        model: IDENTITY,
        normal: IDENTITY,
        color: rgba(color),
        texture,
        sampling: Default::default(),
        uv_set: 0,
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
        double_sided: true,
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
            pick_capture: None,
            depth_background: Default::default(),
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
fn metal_scene3d_composites_depth_clipping_opacity_and_native_paint_order() -> anyhow::Result<()> {
    objc::rc::autoreleasepool(|| {
        let mut renderer =
            MetalRenderer::new_headless(Arc::new(Mutex::new(InstanceBufferPool::default())));
        assert!(renderer.scene3d_support().is_supported());
        let region = bounds(0., 0., 64., 64.);
        let mut scene = Scene::default();
        scene.insert_primitive(quad(region, 0x00ff00ff));
        scene.insert_primitive(Primitive::SubtreeLayer(layer(
            region,
            Scene::default(),
            vec![
                mesh(0.7, 0xff0000ff, MeshTexture3d::None),
                mesh(0.2, 0x0000ffff, MeshTexture3d::None),
            ],
            0.5,
        )));
        scene.insert_primitive(quad(bounds(28., 28., 8., 8.), 0xffffffff));
        let mut second = layer(
            bounds(64., 0., 64., 64.),
            Scene::default(),
            vec![mesh(0.2, 0xff0000ff, MeshTexture3d::None)],
            1.,
        );
        second.composite.content_mask.bounds = bounds(64., 0., 32., 64.);
        scene.insert_primitive(Primitive::SubtreeLayer(second));
        scene.finish();
        let pixels =
            renderer.render_scene_to_image(&scene, size(DevicePixels(128), DevicePixels(64)))?;
        assert_eq!(pixels.get_pixel(32, 32).0, [255, 255, 255, 255]);
        let mixed = pixels.get_pixel(20, 32).0;
        assert!(
            mixed[0] < 3 && (126..=129).contains(&mixed[1]) && (126..=129).contains(&mixed[2]),
            "{mixed:?}"
        );
        assert_eq!(pixels.get_pixel(80, 32).0, [255, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(104, 32).0, [0, 0, 0, 255]);
        assert_eq!(pixels.get_pixel(2, 2).0, [0, 255, 0, 255]);
        Ok(())
    })
}

#[test]
fn metal_scene3d_shares_atlas_captures_picking_and_cache_lifecycle() -> anyhow::Result<()> {
    use gpui::{PlatformAtlas, Scene3dPickCapture};
    use gpui_wgpu::{WgpuContext, WgpuScene3dPickFrame};
    objc::rc::autoreleasepool(|| {
        let pool = Arc::new(Mutex::new(InstanceBufferPool::default()));
        let mut renderer = MetalRenderer::new_headless(pool.clone());
        let atlas = renderer.sprite_atlas().clone();
        let context = atlas
            .renderer_context()
            .unwrap()
            .downcast::<WgpuContext>()
            .unwrap();
        let sibling = MetalRenderer::new_headless(pool);
        let sibling_context = sibling
            .sprite_atlas()
            .renderer_context()
            .unwrap()
            .downcast::<WgpuContext>()
            .unwrap();
        assert!(Arc::ptr_eq(&context.device, &sibling_context.device));
        assert!(Arc::ptr_eq(&context.queue, &sibling_context.queue));
        drop(sibling);
        let mut source = Scene::default();
        source.insert_primitive(quad(bounds(0., 0., 64., 32.), 0xff0000ff));
        source.insert_primitive(quad(bounds(0., 32., 64., 32.), 0x0000ffff));
        let capture = Scene3dPickCapture::new(64 * 64 * 16);
        let mut viewport = layer(
            bounds(0., 0., 64., 64.),
            source,
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Subtree)],
            1.,
        );
        Arc::make_mut(viewport.scene3d.as_mut().unwrap()).pick_capture = Some(capture.clone());
        let frame = viewport.scene3d.as_ref().unwrap().clone();
        let scene = scene(viewport);
        let first =
            renderer.render_scene_to_image(&scene, size(DevicePixels(64), DevicePixels(64)))?;
        assert_eq!(first.get_pixel(32, 20).0, [255, 0, 0, 255]);
        assert_eq!(first.get_pixel(32, 44).0, [0, 0, 255, 255]);
        assert!(
            capture
                .read::<WgpuScene3dPickFrame>()
                .unwrap()
                .unwrap()
                .matches_frame(&frame)
        );
        let second =
            renderer.render_scene_to_image(&scene, size(DevicePixels(64), DevicePixels(64)))?;
        assert_eq!(first, second);
        assert!(
            renderer
                .scene3d_output_cache_stats()
                .unwrap()
                .retained_bytes
                > 0
        );
        renderer.clear_scene3d_caches();
        assert_eq!(
            renderer
                .scene3d_output_cache_stats()
                .unwrap()
                .retained_bytes,
            0
        );
        assert!(atlas.renderer_context().is_some());
        renderer.set_scene3d_output_cache_budget(0);
        let resized =
            renderer.render_scene_to_image(&scene, size(DevicePixels(96), DevicePixels(80)))?;
        assert_eq!(resized.get_pixel(32, 20).0, [255, 0, 0, 255]);
        assert_eq!(resized.get_pixel(32, 44).0, [0, 0, 255, 255]);
        Ok(())
    })
}

#[test]
fn metal_scene3d_atlas_uploads_reach_native_sprites_and_transparent_meshes() -> anyhow::Result<()> {
    use gpui::{PlatformAtlas, PolychromeSprite};
    objc::rc::autoreleasepool(|| {
        let mut renderer =
            MetalRenderer::new_headless(Arc::new(Mutex::new(InstanceBufferPool::default())));
        renderer.update_transparency(true);
        let atlas = renderer.sprite_atlas().clone();
        let key = gpui::RenderImageParams {
            image_id: gpui::ImageId(913),
            frame_index: 0,
        }
        .into();
        let tile = atlas
            .get_or_insert_with(&key, &mut || {
                Ok(Some((
                    size(DevicePixels(2), DevicePixels(2)),
                    std::borrow::Cow::Owned([0, 0, 255, 255].repeat(4)),
                )))
            })?
            .unwrap();
        let region = bounds(0., 0., 16., 16.);
        let mut scene = Scene::default();
        scene.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 1.,
            bounds: region,
            clip_bounds: region,
            content_mask: ContentMask { bounds: region },
            corner_radii: Default::default(),
            tile,
            transformation: Default::default(),
        });
        scene.finish();
        let native =
            renderer.render_scene_to_image(&scene, size(DevicePixels(96), DevicePixels(64)))?;
        assert_eq!(native.get_pixel(8, 8).0, [255, 0, 0, 255]);
        scene.insert_primitive(Primitive::SubtreeLayer(layer(
            bounds(32., 0., 64., 64.),
            Scene::default(),
            vec![mesh(0.2, 0xffffffff, MeshTexture3d::Image(tile))],
            0.5,
        )));
        scene.finish();
        let mixed =
            renderer.render_scene_to_image(&scene, size(DevicePixels(96), DevicePixels(64)))?;
        assert_eq!(mixed.get_pixel(8, 8).0, [255, 0, 0, 255]);
        assert_eq!(mixed.get_pixel(64, 32).0, [128, 0, 0, 128]);
        assert_eq!(mixed.get_pixel(24, 32).0, [0, 0, 0, 0]);
        renderer.clear_scene3d_caches();
        assert_eq!(
            atlas.get_or_insert_with(&key, &mut || anyhow::bail!("live atlas tile was cleared"))?,
            Some(tile)
        );
        Ok(())
    })
}

#[test]
fn core_video_surfaces_are_sampled_in_subtree_captures() -> anyhow::Result<()> {
    use core_foundation::{base::TCFType, dictionary::CFDictionary, string::CFString};
    use core_video::pixel_buffer::{
        CVPixelBuffer, kCVPixelBufferIOSurfacePropertiesKey, kCVPixelFormatType_32BGRA,
        kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    };
    use gpui::{
        CoreVideoHandle, PaintSurface, SurfaceColorInfo, SurfaceFormat, SurfaceFrame, SurfaceHandle,
    };
    objc::rc::autoreleasepool(|| {
        let attributes = CFDictionary::from_CFType_pairs(&[(
            unsafe { CFString::wrap_under_get_rule(kCVPixelBufferIOSurfacePropertiesKey) },
            CFDictionary::<CFString, CFString>::from_CFType_pairs(&[]).as_CFType(),
        )]);
        let mut renderer =
            MetalRenderer::new_headless(Arc::new(Mutex::new(InstanceBufferPool::default())));
        let region = bounds(0., 0., 16., 16.);
        let handle = SurfaceHandle::new();
        for sequence in 0..4 {
            let nv12 = sequence % 2 == 1;
            let buffer = CVPixelBuffer::new(
                if nv12 {
                    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
                } else {
                    kCVPixelFormatType_32BGRA
                },
                8,
                8,
                Some(&attributes),
            )
            .unwrap();
            assert_eq!(buffer.lock_base_address(0), 0);
            unsafe {
                if nv12 {
                    for row in 0..8 {
                        std::ptr::write_bytes(
                            buffer
                                .get_base_address_of_plane(0)
                                .cast::<u8>()
                                .add(row * buffer.get_bytes_per_row_of_plane(0)),
                            235,
                            8,
                        );
                    }
                    for row in 0..4 {
                        std::ptr::write_bytes(
                            buffer
                                .get_base_address_of_plane(1)
                                .cast::<u8>()
                                .add(row * buffer.get_bytes_per_row_of_plane(1)),
                            128,
                            8,
                        );
                    }
                } else {
                    for y in 0..8 {
                        for x in 0..8 {
                            let color = if x < 4 {
                                [0, 0, 255, 255]
                            } else {
                                [0, 255, 0, 255]
                            };
                            std::ptr::copy_nonoverlapping(
                                color.as_ptr(),
                                buffer
                                    .get_base_address()
                                    .cast::<u8>()
                                    .add(y * buffer.get_bytes_per_row() + x * 4),
                                4,
                            );
                        }
                    }
                }
            }
            assert_eq!(buffer.unlock_base_address(0), 0);
            let frame = SurfaceFrame::from_core_video(
                handle.clone(),
                sequence,
                Bounds::new(
                    point(DevicePixels(0), DevicePixels(0)),
                    size(DevicePixels(4), DevicePixels(8)),
                ),
                size(DevicePixels(4), DevicePixels(8)),
                if nv12 {
                    SurfaceFormat::Nv12
                } else {
                    SurfaceFormat::Bgra8
                },
                unsafe { CoreVideoHandle::new(buffer.clone()) },
                SurfaceColorInfo::default(),
            )?;
            let mut content = Scene::default();
            content.insert_primitive(PaintSurface {
                order: 0,
                bounds: region,
                clip_bounds: region,
                content_mask: ContentMask { bounds: region },
                corner_radii: Default::default(),
                opacity: 1.,
                source: frame.into(),
            });
            let mut capture = layer(region, content, vec![], 1.);
            capture.scene3d = None;
            let captured = scene(capture);
            let pixels = renderer
                .render_scene_to_image(&captured, size(DevicePixels(16), DevicePixels(16)))?;
            let center = pixels.get_pixel(8, 8).0;
            if nv12 {
                assert!(center[..3].iter().all(|c| *c >= 250), "{center:?}");
            } else {
                assert_eq!(center, [255, 0, 0, 255]);
            }
            if nv12 {
                let mut legacy = Scene::default();
                legacy.insert_primitive(PaintSurface {
                    order: 0,
                    bounds: region,
                    clip_bounds: region,
                    content_mask: ContentMask { bounds: region },
                    corner_radii: Default::default(),
                    opacity: 1.,
                    source: buffer.into(),
                });
                let mut capture = layer(region, legacy, vec![], 1.);
                capture.scene3d = None;
                let pixels = renderer.render_scene_to_image(
                    &scene(capture),
                    size(DevicePixels(16), DevicePixels(16)),
                )?;
                assert!(pixels.get_pixel(8, 8).0[..3].iter().all(|c| *c >= 250));
            }
        }
        Ok(())
    })
}

#[test]
fn captured_backdrop_blur_samples_preceding_content() -> anyhow::Result<()> {
    objc::rc::autoreleasepool(|| {
        let mut renderer =
            MetalRenderer::new_headless(Arc::new(Mutex::new(InstanceBufferPool::default())));
        let region = bounds(0., 0., 64., 32.);
        let mut content = Scene::default();
        content.insert_primitive(quad(region, 0x000000ff));
        content.insert_primitive(quad(bounds(0., 0., 32., 32.), 0xffffffff));
        content.insert_primitive(gpui::BackdropBlur {
            order: 0,
            bounds: region,
            content_mask: ContentMask { bounds: region },
            corner_radii: Default::default(),
            blur_radius: ScaledPixels(6.),
            opacity: 1.,
            shader: None,
            uniforms: Default::default(),
            time: 0.,
            pointer: point(0., 0.),
            pointer_active: false,
        });
        let mut capture = layer(region, content, vec![], 1.);
        capture.scene3d = None;
        let image = renderer
            .render_scene_to_image(&scene(capture), size(DevicePixels(64), DevicePixels(32)))?;
        assert!(image.get_pixel(28, 16).0[0] < 250);
        assert!(image.get_pixel(36, 16).0[0] > 5);
        assert!(image.get_pixel(8, 16).0[0] > 245);
        assert!(image.get_pixel(56, 16).0[0] < 10);
        Ok(())
    })
}
