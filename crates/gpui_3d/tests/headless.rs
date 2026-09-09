#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use gpui::rgb;
use gpui_3d::{Camera, HeadlessRenderer, Material, Mesh, Node, Scene3dOutputConfig, SceneGraph};

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
