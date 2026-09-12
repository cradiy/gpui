#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use std::time::{Duration, Instant};

use gpui::rgb;
use gpui_3d::{
    Camera, HeadlessRenderer, Material, Mesh, MeshPass, Object, Scene, Scene3dChannels,
    Scene3dOutputConfig, Scene3dPixels,
};
use gpui_3d_effects::LightSweep;

fn read(
    renderer: &mut HeadlessRenderer,
    scene: &Scene,
    samples: u32,
) -> anyhow::Result<Scene3dPixels> {
    let frame = renderer.render(
        scene,
        Scene3dOutputConfig {
            size: [192, 192],
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
fn sweep_moves_without_changing_coverage_or_lighting_occluders() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let sweep = LightSweep::new(renderer.context().clone())?
        .range([-0.6, 0.6])
        .width(0.22);
    let cube = Mesh::cube();
    let colors = cube
        .vertices()
        .iter()
        .map(|v| [1., 1., 1., if v.position[1] > 0. { 0. } else { 1. }])
        .collect();
    let masked = cube.with_vertex_colors(colors)?;
    let scene = |pass: Option<MeshPass>, mask: bool| {
        Scene::new()
            .camera(Camera::orbit(0., 0., 3.))
            .object(Object::new(
                if mask { masked.clone() } else { cube.clone() },
                Material::color(rgb(0x304354)).unlit(true).mesh_passes(pass),
            ))
            .object(
                Object::new(cube.clone(), Material::color(rgb(0x805038)).unlit(true))
                    .scale([0.2, 0.8, 0.2])
                    .position([0., 0., 0.9]),
            )
    };
    for mask in [false, true] {
        for samples in [1, 4] {
            let baseline = read(&mut renderer, &scene(None, mask), samples)?;
            let mut centers = Vec::new();
            for progress in [0., 0.3, 0.7, 1.] {
                let lit = read(
                    &mut renderer,
                    &scene(Some(sweep.pass(progress)?), mask),
                    samples,
                )?;
                assert_eq!(lit.object_ids, baseline.object_ids);
                assert_eq!(lit.linear_depth, baseline.linear_depth);
                assert_eq!(lit.world_normals, baseline.world_normals);
                let mut energy = 0.;
                let mut weighted_x = 0.;
                let ids = baseline.object_ids.as_ref().unwrap();
                for (index, (before, after)) in baseline
                    .linear_rgba
                    .as_ref()
                    .unwrap()
                    .iter()
                    .zip(lit.linear_rgba.as_ref().unwrap())
                    .enumerate()
                {
                    assert!(after.iter().all(|value| value.is_finite()));
                    let difference = (0..3).map(|i| (after[i] - before[i]).abs()).sum::<f32>();
                    // Interior background and foreground samples must not receive the pass.
                    let x = index % 192;
                    let y = index / 192;
                    if x > 0
                        && x < 191
                        && y > 0
                        && y < 191
                        && ids[index] != 1
                        && [index - 1, index + 1, index - 192, index + 192]
                            .into_iter()
                            .all(|neighbor| ids[neighbor] == ids[index])
                    {
                        assert!(difference < 0.002, "light leaked onto ID {}", ids[index]);
                    }
                    energy += difference;
                    weighted_x += difference * x as f32;
                }
                if progress == 0. || progress == 1. {
                    assert!(
                        energy < 0.01,
                        "sweep endpoints should leave the surface unchanged"
                    );
                } else {
                    assert!(energy > 5., "sweep did not illuminate the surface");
                    centers.push(weighted_x / energy);
                }
            }
            assert!(
                centers[1] > centers[0] + 10.,
                "light band did not travel rightward: {centers:?}"
            );
        }
    }
    for invalid in [
        sweep.clone().width(0.),
        sweep.clone().direction([0.; 3]),
        sweep.clone().range([1., -1.]),
        sweep.clone().intensity(f32::NAN),
    ] {
        assert!(invalid.pass(0.5).is_err());
    }
    assert!(sweep.pass(f32::INFINITY).is_err());
    Ok(())
}
