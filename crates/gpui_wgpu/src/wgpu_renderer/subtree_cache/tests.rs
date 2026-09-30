use super::*;
use crate::{WgpuContext, WgpuExternalRenderTarget, WgpuExternalRendererConfig};
use gpui::{Bounds, ContentMask, DevicePixels, EffectQuad, Quad, ScaledPixels, point, rgba, size};
use std::rc::Rc;

fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
    Bounds::new(
        point(ScaledPixels(x), ScaledPixels(y)),
        size(ScaledPixels(width), ScaledPixels(height)),
    )
}

fn layer(source: Scene, offset: f32) -> SubtreeLayer {
    let bounds = bounds(0., 0., 64., 64.);
    SubtreeLayer {
        scene: Rc::new(source),
        second_scene: None,
        scene3d: None,
        intermediate_effects: Default::default(),
        composite: EffectQuad {
            order: 0,
            bounds,
            effect_bounds: bounds,
            transformation: Default::default(),
            content_mask: ContentMask { bounds },
            shader: gpui_effects::transform_group_shader(),
            uniforms: gpui::EffectUniforms::default()
                .with_slot(0, [1., 0., -offset, 0.])
                .with_slot(1, [0., 1., 0., 0.]),
            time: 0.,
            corner_radii: Default::default(),
            opacity: 1.,
            image_tile: None,
            second_image_tile: None,
            third_image_tile: None,
            fourth_image_tile: None,
        },
    }
}

fn scene(color: u32, offset: f32, nested: bool, siblings: bool) -> Scene {
    let mut source = Scene::default();
    let rect = bounds(8., 8., 16., 16.);
    source.insert_primitive(Quad {
        bounds: rect,
        content_mask: ContentMask { bounds: rect },
        background: rgba(color).into(),
        ..Default::default()
    });
    source.finish();
    if nested {
        let mut parent = Scene::default();
        parent.insert_primitive(gpui::Primitive::SubtreeLayer(layer(source, 0.)));
        parent.finish();
        source = parent;
    }
    let layer = layer(source, offset);
    let mut result = Scene::default();
    result.insert_primitive(gpui::Primitive::SubtreeLayer(layer.clone()));
    if siblings {
        result.insert_primitive(gpui::Primitive::SubtreeLayer(layer));
    }
    result.finish();
    result
}

fn texture(context: &WgpuContext, width: u32) -> wgpu::Texture {
    context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("subtree cache regression"),
        size: wgpu::Extent3d {
            width,
            height: width,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

fn renderer(context: &WgpuContext) -> anyhow::Result<WgpuRenderer> {
    WgpuRenderer::new_external(
        context,
        WgpuExternalRendererConfig {
            size: size(DevicePixels(64), DevicePixels(64)),
            format: wgpu::TextureFormat::Rgba8Unorm,
            alpha_mode: wgpu::CompositeAlphaMode::PreMultiplied,
            target_usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        },
    )
}

fn draw(renderer: &mut WgpuRenderer, scene: &Scene, target: &wgpu::Texture) -> usize {
    let view = target.create_view(&Default::default());
    for _ in 0..10 {
        if renderer.draw_external(scene, target, &view, wgpu::Color::TRANSPARENT) {
            return renderer.resources().subtree_cache.hits.get();
        }
    }
    panic!("scene did not fit after retries");
}

fn pixels(context: &WgpuContext, texture: &wgpu::Texture) -> Vec<u8> {
    let stride = (texture.width() * 4).div_ceil(256) * 256;
    let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride * texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = context.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    context.queue.submit([encoder.finish()]);
    let (tx, rx) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| tx.send(result).unwrap());
    context
        .device
        .poll(wgpu::PollType::wait_indefinitely())
        .unwrap();
    rx.recv().unwrap().unwrap();
    let data = buffer.slice(..).get_mapped_range().unwrap();
    let result = data.to_vec();
    drop(data);
    buffer.unmap();
    result
}

#[test]
#[ignore = "requires a GPU adapter"]
fn raster_capture_preserves_offset_alpha_and_nested_density() -> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = renderer(&context)?;
    let target = texture(&context, 64);
    for (density, nested) in [
        (2., false),
        (2., false),
        (3., false),
        (2., true),
        (1., false),
    ] {
        let total_density = density * if nested { 2. } else { 1. };
        let mut source = Scene::default();
        source.raster_scale = Some(if nested { 2. } else { density });
        let rect = bounds(12., 14., 8., 6.).map(|p| ScaledPixels(p.0 * total_density));
        source.insert_primitive(Quad {
            bounds: rect,
            content_mask: ContentMask { bounds: rect },
            background: rgba(0xff000080).into(),
            ..Default::default()
        });
        source.finish();
        if nested {
            let mut inner = layer(source, 0.);
            inner.composite.bounds = bounds(8., 8., 48., 48.).map(|p| ScaledPixels(p.0 * density));
            inner.composite.effect_bounds = inner.composite.bounds;
            inner.composite.content_mask.bounds = inner.composite.bounds;
            source = Scene::default();
            source.raster_scale = Some(density);
            source.insert_primitive(gpui::Primitive::SubtreeLayer(inner));
            source.finish();
        }
        let mut outer = layer(source, 0.);
        outer.composite.bounds = bounds(8., 8., 48., 48.);
        outer.composite.effect_bounds = outer.composite.bounds;
        outer.composite.content_mask.bounds = outer.composite.bounds;
        outer.composite.uniforms = gpui::EffectUniforms::default()
            .with_slot(0, [0.5, 0., 0., 0.])
            .with_slot(1, [0., 0.5, 0., 0.]);
        let mut scene = Scene::default();
        scene.insert_primitive(gpui::Primitive::SubtreeLayer(outer));
        scene.finish();
        draw(&mut renderer, &scene, &target);
        let output = pixels(&context, &target);
        let sample = |x: usize, y: usize| &output[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        assert_eq!(
            sample(22, 25),
            &[128, 0, 0, 128],
            "density={density}, nested={nested}"
        );
        for (x, y) in [(10, 10), (38, 25), (22, 38)] {
            assert_eq!(sample(x, y), &[0; 4]);
        }
        {
            let capture = &renderer.resources().ui_captures[0];
            assert_eq!(capture.texture.width(), (48. * density) as u32);
            assert_eq!(capture.renderer.capture_origin, [8. * density; 2]);
            assert!(renderer.resources().subtree_textures.is_empty());
            if nested {
                assert_eq!(
                    capture.renderer.resources().ui_captures[0].texture.width(),
                    192
                );
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn regional_capture_matches_full_frame_pixels_after_moving_crop() -> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut full_renderer = renderer(&context)?;
    full_renderer.update_drawable_size(size(DevicePixels(128), DevicePixels(128)));
    let full_target = texture(&context, 128);
    let mut source = Scene::default();
    source.raster_scale = Some(2.);
    let rect = bounds(32., 36., 28., 24.);
    let clip = ContentMask {
        bounds: bounds(0., 0., 128., 128.),
    };
    source.insert_primitive(gpui::Shadow {
        order: 0,
        bounds: rect,
        content_mask: clip,
        color: rgba(0x00000080).into(),
        blur_radius: ScaledPixels(3.),
        corner_radii: gpui::Corners::all(ScaledPixels(5.)),
        element_bounds: rect,
        element_corner_radii: gpui::Corners::all(ScaledPixels(5.)),
        inset: 0,
        pad: 0,
    });
    source.insert_primitive(Quad {
        bounds: rect,
        content_mask: clip,
        background: rgba(0xff000080).into(),
        corner_radii: gpui::Corners::all(ScaledPixels(5.)),
        ..Default::default()
    });
    source.insert_primitive(gpui::Underline {
        order: 0,
        pad: 0,
        bounds: bounds(40., 66., 36., 6.),
        content_mask: clip,
        color: rgba(0x00ff00ff).into(),
        thickness: ScaledPixels(2.),
        wavy: true.into(),
    });
    let mut path = gpui::PathBuilder::fill();
    path.move_to(point(gpui::px(25.), gpui::px(40.)));
    path.line_to(point(gpui::px(35.), gpui::px(55.)));
    path.line_to(point(gpui::px(15.), gpui::px(55.)));
    path.close();
    let mut path = path.build()?.scale(2.);
    path.color = rgba(0x2244ffff).into();
    path.content_mask = clip;
    source.insert_primitive(path);
    let mut effect = layer(Scene::default(), 0.).composite;
    effect.bounds = bounds(80., 24., 20., 20.);
    effect.effect_bounds = effect.bounds;
    effect.content_mask = clip;
    effect.shader = gpui::EffectShader::wgsl(
        "fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return vec4<f32>(input.position / 128.0, 0.0, 1.0); }",
    );
    source.insert_primitive(effect);
    source.finish();
    draw(&mut full_renderer, &source, &full_target);
    let reference = pixels(&context, &full_target);
    let source = Rc::new(source);
    let mut cropped_renderer = renderer(&context)?;
    let target = texture(&context, 64);
    for crop in [
        bounds(8., 8., 48., 48.),
        bounds(10., 6., 48., 48.),
        bounds(10.25, 6.75, 40., 45.),
        bounds(-4., -3., 48., 48.),
    ] {
        let mut outer = layer(Scene::default(), 0.);
        outer.scene = source.clone();
        outer.composite.bounds = crop;
        outer.composite.effect_bounds = crop;
        outer.composite.content_mask.bounds = crop;
        let mut scene = Scene::default();
        scene.insert_primitive(gpui::Primitive::SubtreeLayer(outer));
        scene.finish();
        draw(&mut cropped_renderer, &scene, &target);
        let capture = &cropped_renderer.resources().ui_captures[0];
        let actual = pixels(&context, &capture.texture);
        let [left, top] = capture.renderer.capture_origin.map(|value| value as usize);
        let width = capture.texture.width() as usize;
        let stride = (width * 4).div_ceil(256) * 256;
        for row in 0..capture.texture.height() as usize {
            let expected =
                &reference[(row + top) * 512 + left * 4..(row + top) * 512 + (left + width) * 4];
            let actual = &actual[row * stride..row * stride + width * 4];
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, b)| a.abs_diff(*b) <= 1),
                "row {row}, crop {crop:?}"
            );
        }
        assert!(cropped_renderer.resources().subtree_textures.is_empty());
        assert!(
            cropped_renderer
                .resources()
                .path_intermediate_texture
                .is_none()
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn nested_capture_can_translate_source_from_outside_parent_crop() -> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = renderer(&context)?;
    let target = texture(&context, 64);
    for (density, inner_density) in [(1., Some(2.)), (2., Some(2.)), (1., None), (2., None)] {
        let mut source = Scene::default();
        source.raster_scale = inner_density;
        let rect = bounds(44., 16., 4., 4.)
            .map(|p| ScaledPixels(p.0 * density * inner_density.unwrap_or(1.)));
        source.insert_primitive(Quad {
            bounds: rect,
            content_mask: ContentMask { bounds: rect },
            background: rgba(0xff0000ff).into(),
            ..Default::default()
        });
        source.finish();
        let mut inner = layer(source, -30. * density);
        inner.composite.bounds = bounds(0., 0., 64., 64.).map(|p| ScaledPixels(p.0 * density));
        inner.composite.effect_bounds = inner.composite.bounds;
        inner.composite.content_mask.bounds = inner.composite.bounds;
        let mut parent = Scene::default();
        parent.raster_scale = Some(density);
        parent.insert_primitive(gpui::Primitive::SubtreeLayer(inner));
        parent.finish();
        let mut outer = layer(parent, 0.);
        outer.composite.bounds = bounds(8., 8., 24., 24.);
        outer.composite.effect_bounds = outer.composite.bounds;
        outer.composite.content_mask.bounds = outer.composite.bounds;
        let mut scene = Scene::default();
        scene.insert_primitive(gpui::Primitive::SubtreeLayer(outer));
        scene.finish();
        draw(&mut renderer, &scene, &target);
        let output = pixels(&context, &target);
        assert_eq!(
            &output[(17 * 64 + 15) * 4..(17 * 64 + 15) * 4 + 4],
            &[255, 0, 0, 255],
            "density={density}, inner={inner_density:?}"
        );
        if inner_density.is_some() {
            let capture = &renderer.resources().ui_captures[0];
            assert_eq!(capture.texture.width(), (24. * density) as u32);
            assert_eq!(
                capture.renderer.resources().ui_captures[0].texture.width(),
                (128. * density) as u32
            );
        } else if density > 1. {
            assert_eq!(renderer.resources().ui_captures[0].texture.width(), 128);
        } else {
            assert!(renderer.resources().ui_captures.is_empty());
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn capture_diagnostics_track_reuse_nested_storage_and_release() -> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = renderer(&context)?;
    let target = texture(&context, 64);
    let make_scene = |color, offset| {
        let mut source = Scene::default();
        source.raster_scale = Some(2.);
        let rect = bounds(8., 8., 16., 16.);
        source.insert_primitive(Quad {
            bounds: rect,
            content_mask: ContentMask { bounds: rect },
            background: rgba(color).into(),
            ..Default::default()
        });
        source.finish();
        let mut outer = Scene::default();
        outer.raster_scale = Some(2.);
        outer.insert_primitive(gpui::Primitive::SubtreeLayer(layer(source, 0.)));
        outer.finish();
        let mut scene = Scene::default();
        scene.insert_primitive(gpui::Primitive::SubtreeLayer(layer(outer, offset)));
        scene.finish();
        scene
    };
    draw(&mut renderer, &make_scene(0xff0000ff, 0.), &target);
    let first = renderer.diagnostics().unwrap();
    assert_eq!(first.capture_cache.hits, 0);
    assert_eq!(first.capture_cache.misses, 2);
    assert_eq!(first.capture_textures.len(), 2);
    let first_bytes: u64 = first
        .capture_textures
        .iter()
        .map(|t| t.estimated_bytes)
        .sum();
    assert!(first_bytes > 0);
    for texture in &first.capture_textures {
        assert_eq!(
            texture.estimated_bytes,
            u64::from(texture.width) * u64::from(texture.height) * 4
        );
        assert_eq!(texture.raster_scale, Some(2.));
    }
    draw(&mut renderer, &make_scene(0xff0000ff, 8.), &target);
    let reused = renderer.diagnostics().unwrap();
    assert_eq!(
        reused.capture_cache.hits, 1,
        "a reused parent does not visit the child"
    );
    assert_eq!(reused.capture_cache.misses, 0);
    assert_eq!(
        reused
            .capture_textures
            .iter()
            .map(|t| t.estimated_bytes)
            .sum::<u64>(),
        first_bytes
    );
    draw(&mut renderer, &make_scene(0x0000ffff, 8.), &target);
    assert_eq!(renderer.diagnostics().unwrap().capture_cache.misses, 2);
    draw(&mut renderer, &Scene::default(), &target);
    let empty = renderer.diagnostics().unwrap();
    assert!(empty.capture_textures.is_empty());
    assert_eq!(empty.capture_cache.hits + empty.capture_cache.misses, 0);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn subtree_capture_cache_reuses_only_submitted_unchanged_single_writer_textures()
-> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = renderer(&context)?;
    let target = texture(&context, 64);
    assert_eq!(
        draw(&mut renderer, &scene(0xff0000ff, 0., false, false), &target),
        0
    );
    let original_texture = renderer.resources().subtree_textures[0].clone();
    // Fresh scene allocations and changed composite uniforms must still reuse the source.
    assert_eq!(
        draw(
            &mut renderer,
            &scene(0xff0000ff, 16., false, false),
            &target
        ),
        1
    );
    assert_eq!(renderer.resources().subtree_textures[0], original_texture);
    let image = pixels(&context, &target);
    assert_eq!(
        &image[(16 * 64 + 32) * 4..(16 * 64 + 32) * 4 + 4],
        &[255, 0, 0, 255]
    );
    assert_eq!(
        &image[(16 * 64 + 12) * 4..(16 * 64 + 12) * 4 + 4],
        &[0, 0, 0, 0]
    );
    assert_eq!(
        draw(
            &mut renderer,
            &scene(0x0000ffff, 16., false, false),
            &target
        ),
        0
    );
    let image = pixels(&context, &target);
    assert_eq!(
        &image[(16 * 64 + 32) * 4..(16 * 64 + 32) * 4 + 4],
        &[0, 0, 255, 255]
    );

    // An encoded but discarded update cannot become a cache hit on retry.
    let green = scene(0x00ff00ff, 0., false, false);
    let view = target.create_view(&Default::default());
    let mut discarded = context.device.create_command_encoder(&Default::default());
    assert!(matches!(
        renderer.encode_external_scene(
            &green,
            WgpuExternalRenderTarget {
                texture: &target,
                view: &view,
                command_encoder: &mut discarded,
            },
            true
        )?,
        super::super::SceneEncoding::Complete
    ));
    renderer.commit_encoded_scene(false);
    drop(discarded);
    assert_eq!(draw(&mut renderer, &green, &target), 0);
    assert_eq!(draw(&mut renderer, &green, &target), 1);

    // Nested source textures can be reused independently; sibling scratch reuse cannot.
    assert_eq!(
        draw(&mut renderer, &scene(0xff0000ff, 0., true, false), &target),
        0
    );
    assert_eq!(
        draw(&mut renderer, &scene(0xff0000ff, 8., true, false), &target),
        1
    );
    assert_eq!(
        draw(&mut renderer, &scene(0xff0000ff, 8., true, true), &target),
        0
    );
    assert_eq!(
        draw(&mut renderer, &scene(0xff0000ff, 8., true, true), &target),
        0
    );
    assert_eq!(draw(&mut renderer, &green, &target), 0);
    assert_eq!(draw(&mut renderer, &green, &target), 1);

    // Intermediate passes overwrite the same scratch targets as source captures.
    let mut processed = scene(0xff0000ff, 0., false, false);
    let composite = &processed.subtree_layers[0].composite;
    processed.subtree_layers[0].intermediate_effects = vec![gpui::SubtreeEffectPass {
        shader: composite.shader.clone(),
        uniforms: composite.uniforms,
        time: 0.,
        images: Default::default(),
        bloom: None,
        feedback: None,
        distance_field: None,
        particles: None,
        particle_transition: None,
    }]
    .into();
    assert_eq!(draw(&mut renderer, &processed, &target), 0);
    assert_eq!(draw(&mut renderer, &processed, &target), 0);
    assert_eq!(draw(&mut renderer, &green, &target), 0);
    assert_eq!(draw(&mut renderer, &green, &target), 1);
    let image = pixels(&context, &target);
    assert_eq!(
        &image[(16 * 64 + 16) * 4..(16 * 64 + 16) * 4 + 4],
        &[0, 255, 0, 255]
    );

    renderer.is_bgr = !renderer.is_bgr;
    assert_eq!(draw(&mut renderer, &green, &target), 0);
    // Caller-owned commands can overwrite scratch textures after a later submitted frame.
    let mut external = context.device.create_command_encoder(&Default::default());
    assert!(renderer.encode_external(
        &scene(0xff0000ff, 0., false, false),
        WgpuExternalRenderTarget {
            texture: &target,
            view: &view,
            command_encoder: &mut external,
        }
    ));
    assert_eq!(draw(&mut renderer, &green, &target), 0);
    context.queue.submit([external.finish()]);
    assert_eq!(draw(&mut renderer, &green, &target), 0);
    let image = pixels(&context, &target);
    assert_eq!(
        &image[(16 * 64 + 16) * 4..(16 * 64 + 16) * 4 + 4],
        &[0, 255, 0, 255]
    );

    renderer.update_drawable_size(size(DevicePixels(80), DevicePixels(80)));
    let resized = texture(&context, 80);
    assert_eq!(draw(&mut renderer, &green, &resized), 0);
    assert_eq!(draw(&mut renderer, &green, &resized), 1);
    assert_ne!(renderer.resources().subtree_textures[0], original_texture);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter; reports timings without asserting a speed threshold"]
fn subtree_capture_cache_100_quad_transform_benchmark() -> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    eprintln!("Adapter: {:?}", context.adapter.get_info());
    let mut renderer = renderer(&context)?;
    renderer.update_drawable_size(size(DevicePixels(1024), DevicePixels(1024)));
    let target = texture(&context, 1024);
    let mut source = Scene::default();
    for i in 0..100 {
        let rect = bounds((i % 10) as f32 * 100., (i / 10) as f32 * 100., 90., 90.);
        source.insert_primitive(Quad {
            bounds: rect,
            content_mask: ContentMask { bounds: rect },
            background: rgba(0xff0000ff).into(),
            ..Default::default()
        });
    }
    source.finish();
    let sources: Vec<_> = (0..120)
        .map(|i| {
            let mut scene = Scene::default();
            let mut copy = Scene::default();
            for quad in &source.quads {
                copy.insert_primitive(*quad);
            }
            copy.finish();
            let mut layer = layer(copy, (i % 10) as f32);
            let viewport = bounds(0., 0., 1024., 1024.);
            layer.composite.bounds = viewport;
            layer.composite.effect_bounds = viewport;
            layer.composite.content_mask.bounds = viewport;
            let scale = 1. + (i % 20) as f32 * 0.025;
            layer
                .composite
                .uniforms
                .set_slot(0, [1. / scale, 0., -(i % 10) as f32 / scale, 0.]);
            layer
                .composite
                .uniforms
                .set_slot(1, [0., 1. / scale, 0., 0.]);
            scene.insert_primitive(gpui::Primitive::SubtreeLayer(layer));
            scene.finish();
            scene
        })
        .collect();
    for reuse in [false, true] {
        renderer.resources_mut().subtree_cache = SubtreeCaptureCache::default();
        renderer.resources_mut().subtree_cache.external_writes = !reuse;
        let mut samples = Vec::new();
        let mut hits = 0;
        for (i, scene) in sources.iter().enumerate() {
            let start = std::time::Instant::now();
            let reused = draw(&mut renderer, scene, &target);
            context.device.poll(wgpu::PollType::wait_indefinitely())?;
            if i >= 20 {
                samples.push(start.elapsed().as_secs_f64() * 1000.);
                hits += reused;
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "1024x1024 / 100 quads / 100 measured frames / reuse={reuse}: median={:.3}ms p95={:.3}ms source_hits={hits}",
            samples[50], samples[95]
        );
        assert_eq!(hits, if reuse { 100 } else { 0 });
    }
    Ok(())
}
