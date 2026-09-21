use gpui::{
    AtlasKey, Bounds, ContentMask, DevicePixels, ImageId, PathBuilder, PlatformAtlas,
    RenderImageParams, Scene, point, px, rgba, size,
};
use gpui_wgpu::{WgpuAtlas, WgpuContext, WgpuOffscreenRenderer};
use std::{borrow::Cow, collections::VecDeque, sync::Arc};

#[test]
#[ignore = "requires a GPU adapter"]
fn unused_path_targets_are_not_allocated_and_recreated_pixels_match() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(64), DevicePixels(48)))?;
    let empty = Scene::default();
    renderer.render_rgba(&empty)?;
    assert_eq!(renderer.memory_stats().path_texture_bytes, 0);

    let mut builder = PathBuilder::fill();
    builder.add_polygon(
        &[
            point(px(8.), px(8.)),
            point(px(53.), px(13.)),
            point(px(22.), px(39.)),
        ],
        true,
    );
    let mut path = builder.build()?;
    path.content_mask = ContentMask {
        bounds: Bounds::new(point(px(0.), px(0.)), size(px(64.), px(48.))),
    };
    path.color = rgba(0xff8033ff).into();
    let mut scene = Scene::default();
    scene.insert_primitive(path.scale(1.));
    scene.finish();
    let expected = renderer.render_rgba(&scene)?;
    assert!(expected.chunks_exact(4).any(|pixel| pixel[0] > 100));
    let allocated = renderer.memory_stats().path_texture_bytes;
    assert!(allocated >= 64 * 48 * 4);
    renderer.render_rgba(&empty)?;
    assert_eq!(
        renderer.memory_stats().path_texture_bytes,
        allocated,
        "short gaps keep targets warm"
    );
    for _ in 0..120 {
        renderer.render_rgba(&empty)?;
    }
    assert_eq!(renderer.memory_stats().path_texture_bytes, 0);
    assert_eq!(renderer.render_rgba(&scene)?, expected);
    renderer.resize(size(DevicePixels(32), DevicePixels(24)));
    renderer.render_rgba(&empty)?;
    assert_eq!(renderer.memory_stats().path_texture_bytes, 0);
    let bounds = Bounds::new(
        point(gpui::ScaledPixels(0.), gpui::ScaledPixels(0.)),
        size(gpui::ScaledPixels(64.), gpui::ScaledPixels(48.)),
    );
    let mut nested = Scene::default();
    nested.insert_primitive(gpui::Primitive::SubtreeLayer(gpui::SubtreeLayer {
        scene: std::rc::Rc::new(scene),
        second_scene: None,
        scene3d: None,
        intermediate_effects: Arc::default(),
        composite: gpui::EffectQuad {
            order: 0, bounds, effect_bounds: bounds, content_mask: ContentMask { bounds },
            transformation: Default::default(), corner_radii: Default::default(),
            shader: gpui::EffectShader::wgsl_image("fn effect(input: EffectInput, params: EffectParams) -> vec4<f32> { return sample_effect_image(input, input.uv); }"),
            uniforms: Default::default(), time: 0., opacity: 1.,
            image_tile: None, second_image_tile: None, third_image_tile: None, fourth_image_tile: None,
        },
    }));
    renderer.resize(size(DevicePixels(64), DevicePixels(48)));
    let nested_pixels = renderer.render_rgba(&nested)?;
    assert!(
        renderer.memory_stats().path_texture_bytes > 0,
        "nested paths allocate their targets"
    );
    assert!(
        nested_pixels
            .iter()
            .zip(&expected)
            .all(|(a, b)| a.abs_diff(*b) <= 3)
    );
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn instance_buffer_releases_a_past_peak_without_changing_pixels() -> anyhow::Result<()> {
    let mut renderer = WgpuOffscreenRenderer::new(size(DevicePixels(4), DevicePixels(4)))?;
    let baseline = renderer.memory_stats().instance_buffer_bytes;
    let mut scene = Scene::default();
    let bounds = Bounds::new(
        point(gpui::ScaledPixels(0.), gpui::ScaledPixels(0.)),
        size(gpui::ScaledPixels(4.), gpui::ScaledPixels(4.)),
    );
    for _ in 0..(baseline as usize / std::mem::size_of::<gpui::Quad>() + 100) {
        scene.insert_primitive(gpui::Quad {
            bounds,
            content_mask: ContentMask { bounds },
            background: rgba(0x336699ff).into(),
            ..Default::default()
        });
    }
    scene.finish();
    let render = |renderer: &mut WgpuOffscreenRenderer| -> anyhow::Result<Vec<u8>> {
        for _ in 0..4 {
            match renderer.render_rgba(&scene) {
                Ok(pixels) => return Ok(pixels),
                Err(_) => continue,
            }
        }
        anyhow::bail!("instance storage did not accommodate the scene")
    };
    let expected = render(&mut renderer)?;
    assert!(renderer.memory_stats().instance_buffer_bytes > baseline);
    for _ in 0..240 {
        renderer.render_rgba(&Scene::default())?;
    }
    assert_eq!(renderer.memory_stats().instance_buffer_bytes, baseline);
    assert_eq!(render(&mut renderer)?, expected);
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn transient_atlas_frames_are_bounded_and_shared_owners_remain_valid() -> anyhow::Result<()> {
    let context = WgpuContext::new_headless()?;
    let atlas = WgpuAtlas::from_context(&context);
    let mut decoded = VecDeque::new();
    let retained = Arc::new(());
    let key = RenderImageParams {
        image_id: ImageId(1),
        frame_index: 0,
    };
    let pixels = vec![255; 256 * 256 * 4];
    let persistent_key = AtlasKey::Image(key.clone());
    let persistent_tile = atlas
        .get_or_insert_with(&persistent_key, &mut || {
            Ok(Some((
                size(DevicePixels(256), DevicePixels(256)),
                Cow::Borrowed(&pixels),
            )))
        })?
        .unwrap();
    let insert = |params: &RenderImageParams| {
        atlas.get_or_insert_with(&AtlasKey::TransientImage(params.clone()), &mut || {
            Ok(Some((
                size(DevicePixels(256), DevicePixels(256)),
                Cow::Borrowed(&pixels),
            )))
        })
    };
    atlas.retain_image(&key, Arc::downgrade(&retained));
    let retained_tile = insert(&key)?.unwrap();
    assert_ne!(retained_tile, persistent_tile);
    for frame in 2..130 {
        let params = RenderImageParams {
            image_id: ImageId(frame),
            frame_index: 0,
        };
        let owner = Arc::new(());
        atlas.retain_image(&params, Arc::downgrade(&owner));
        insert(&params)?;
        decoded.push_back(owner);
        if decoded.len() > 3 {
            decoded.pop_front();
        }
        atlas.before_frame();
        context.queue.submit([]);
        assert!(atlas.memory_stats().tiles <= 5);
        assert!(
            atlas.memory_stats().texture_bytes <= 8 * 1024 * 1024,
            "expired frames must release reusable atlas space"
        );
        assert_eq!(insert(&key)?.unwrap(), retained_tile);
    }
    // A re-decoded frame with the same ID has another independent owner.
    let second_owner = Arc::new(());
    atlas.retain_image(&key, Arc::downgrade(&second_owner));
    drop(retained);
    decoded.clear();
    atlas.before_frame();
    assert_eq!(atlas.memory_stats().tiles, 2);
    assert_eq!(insert(&key)?.unwrap(), retained_tile);
    drop(second_owner);
    atlas.before_frame();
    assert_eq!(atlas.memory_stats().tiles, 1);
    assert_eq!(
        atlas
            .get_or_insert_with(&persistent_key, &mut || panic!(
                "persistent tile was evicted"
            ))?
            .unwrap(),
        persistent_tile
    );
    atlas.remove(&persistent_key);
    atlas.before_frame();
    assert_eq!(atlas.memory_stats().texture_bytes, 0);
    assert_eq!(atlas.memory_stats().tiles, 0);
    assert_eq!(atlas.memory_stats().pending_upload_bytes, 0);
    Ok(())
}
