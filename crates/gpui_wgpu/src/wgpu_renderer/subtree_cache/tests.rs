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
        source.raster_scale = (total_density > 1.).then_some(if nested { 2. } else { density });
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
        if density > 1. {
            let capture = &renderer.resources().ui_captures[0];
            assert_eq!(capture.texture.width(), (64. * density) as u32);
            if nested {
                assert_eq!(
                    capture.renderer.resources().ui_captures[0].texture.width(),
                    256
                );
            }
        } else {
            assert!(renderer.resources().ui_captures.is_empty());
        }
    }
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
