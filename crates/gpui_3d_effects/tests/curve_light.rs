#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use std::time::{Duration, Instant};

use gpui::rgb;
use gpui_3d::{
    Camera, HeadlessRenderer, Material, Mesh, Object, Scene, Scene3dChannels, Scene3dOutputConfig,
    Scene3dPixels,
};
use gpui_3d_effects::CurveLight;

fn read(
    renderer: &mut HeadlessRenderer,
    scene: &Scene,
    samples: u32,
) -> anyhow::Result<Scene3dPixels> {
    let frame = renderer.render(
        scene,
        Scene3dOutputConfig {
            size: [256, 256],
            channels: Scene3dChannels::all(),
            color_samples: samples,
        },
    )?;
    let mut pending = frame.readback()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(output) = pending.try_read()? {
            return Ok(output.pixels);
        }
        anyhow::ensure!(Instant::now() < deadline, "render readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn curve_flow_keeps_coverage_and_reveal_follows_arc_length() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let curve = CurveLight::new(
        renderer.context().clone(),
        [[-1.5, 0., 0.], [-1.4, 0., 0.], [1.5, 0., 0.]],
    )?
    .width(0.06)?;
    let camera = Camera::orbit(0., 0., 4.);
    for samples in [1, 4] {
        let mut previous_ids = None;
        let mut centers = Vec::new();
        for phase in [0.25, 0.5, 0.75] {
            let pixels = read(
                &mut renderer,
                &Scene::new().camera(camera).object(curve.object(phase)?),
                samples,
            )?;
            if let Some(ids) = previous_ids {
                assert_eq!(pixels.object_ids.as_ref().unwrap(), &ids);
            }
            previous_ids = pixels.object_ids.clone();
            let mut sum = [0f64; 2];
            for (i, rgba) in pixels.linear_rgba.as_ref().unwrap().iter().enumerate() {
                assert!(rgba.iter().all(|v| v.is_finite()));
                let weight = f64::from((rgba[0].max(rgba[1]).max(rgba[2]) - 0.4).max(0.));
                sum[0] += (i % 256) as f64 * weight;
                sum[1] += weight;
            }
            assert!(sum[1] > 1., "missing bright head");
            centers.push(sum[0] / sum[1]);
        }
        assert!(centers[1] - centers[0] > 20.);
        assert!(
            ((centers[1] - centers[0]) - (centers[2] - centers[1])).abs() < 2.,
            "uneven control spacing changed flow speed: {centers:?}"
        );
        let mut previous_mask = vec![0; 256 * 256];
        let mut counts = Vec::new();
        for progress in [0., 0.25, 0.5, 0.75, 1.] {
            let pixels = read(
                &mut renderer,
                &Scene::new().camera(camera).object(curve.reveal(progress)?),
                samples,
            )?;
            let ids = pixels.object_ids.unwrap();
            for (before, after) in previous_mask.iter().zip(&ids) {
                assert!(
                    *before == 0 || *after != 0,
                    "revealing removed an existing surface"
                );
            }
            counts.push(ids.iter().filter(|id| **id != 0).count());
            previous_mask = ids;
        }
        assert_eq!(counts[0], 0);
        assert!(counts.windows(2).all(|pair| pair[1] > pair[0]));
        let fraction = counts[2] as f64 / counts[4] as f64;
        assert!(
            (fraction - 0.5).abs() < 0.04,
            "half reveal covered {fraction}"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn curve_passes_in_front_of_and_behind_scene_geometry() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let curve = CurveLight::new(
        renderer.context().clone(),
        [
            [-1.4, 0.2, -0.8],
            [0.2, 0.2, -0.8],
            [1.1, 0., 0.],
            [0.2, -0.2, 0.8],
            [-1.4, -0.2, 0.8],
        ],
    )?
    .width(0.045)?;
    let camera = Camera::orbit(0., 0., 4.);
    let base = Scene::new()
        .camera(camera)
        .object(Object::new(Mesh::cube(), Material::color(rgb(0x304354))));
    let solid = read(&mut renderer, &base, 4)?.object_ids.unwrap();
    let path = read(
        &mut renderer,
        &Scene::new().camera(camera).object(curve.object(0.5)?),
        4,
    )?
    .object_ids
    .unwrap();
    let combined = read(&mut renderer, &base.object(curve.object(0.5)?), 4)?
        .object_ids
        .unwrap();
    let (mut hidden, mut front) = (0, 0);
    for ((solid, path), combined) in solid.iter().zip(path).zip(combined) {
        if *solid == 1 && path == 1 {
            if combined == 1 {
                hidden += 1;
            }
            if combined == 2 {
                front += 1;
            }
        }
    }
    assert!(
        hidden > 20 && front > 20,
        "{hidden} hidden, {front} in front"
    );
    Ok(())
}
