#![cfg(all(feature = "wgpu", not(target_family = "wasm")))]

use std::time::{Duration, Instant};

use gpui::rgb;
use gpui_3d::{
    AlphaMode, Camera, HeadlessRenderer, Material, Mesh, MeshPass, Object, Scene, Scene3dChannels,
    Scene3dOutputConfig, Scene3dPixels, SphereOptions,
};
use gpui_3d_effects::RimLight;

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

fn difference(before: &[f32; 4], after: &[f32; 4]) -> f32 {
    (0..3).map(|i| (after[i] - before[i]).abs()).sum()
}

#[test]
#[ignore = "requires a GPU adapter"]
fn rim_concentrates_on_curved_edges_without_changing_coverage() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let rim = RimLight::new(renderer.context().clone())?;
    let sphere = Mesh::sphere(SphereOptions {
        radius: 0.75,
        segments: [96, 48],
    })?;
    let masked = sphere.with_vertex_colors(
        sphere
            .vertices()
            .iter()
            .map(|v| [1., 1., 1., if v.position[1] > 0.2 { 0. } else { 1. }])
            .collect(),
    )?;
    let occluder = Mesh::cube();
    for mask in [false, true] {
        let scene = |pass: Option<MeshPass>| {
            Scene::new()
                .camera(Camera::orbit(0., 0., 3.))
                .object(Object::new(
                    if mask { masked.clone() } else { sphere.clone() },
                    Material::color(rgb(0x304354))
                        .unlit(true)
                        .alpha_mode(if mask {
                            AlphaMode::Mask
                        } else {
                            AlphaMode::Opaque
                        })
                        .mesh_passes(pass),
                ))
                .object(
                    Object::new(occluder.clone(), Material::color(rgb(0x805038)).unlit(true))
                        .scale([0.18, 1., 0.18])
                        .position([-0.3, 0., 1.]),
                )
        };
        for samples in [1, 4] {
            let baseline = read(&mut renderer, &scene(None), samples)?;
            let ids = baseline.object_ids.as_ref().unwrap();
            let normals = baseline.world_normals.as_ref().unwrap();
            let mut energies = Vec::new();
            for falloff in [2., 6.] {
                let lit = read(
                    &mut renderer,
                    &scene(Some(rim.clone().falloff(falloff).pass()?)),
                    samples,
                )?;
                assert_eq!(lit.object_ids, baseline.object_ids);
                assert_eq!(lit.linear_depth, baseline.linear_depth);
                assert_eq!(lit.world_normals, baseline.world_normals);
                let mut energy = 0.;
                let mut edge = (0., 0);
                let mut center = (0., 0);
                for (index, (before, after)) in baseline
                    .linear_rgba
                    .as_ref()
                    .unwrap()
                    .iter()
                    .zip(lit.linear_rgba.as_ref().unwrap())
                    .enumerate()
                {
                    assert!(after.iter().all(|v| v.is_finite()));
                    assert!((after[3] - before[3]).abs() < 0.002);
                    let delta = difference(before, after);
                    let x = index % 192;
                    let y = index / 192;
                    let interior = x > 0
                        && x < 191
                        && y > 0
                        && y < 191
                        && [index - 1, index + 1, index - 192, index + 192]
                            .into_iter()
                            .all(|neighbor| ids[neighbor] == ids[index]);
                    if ids[index] != 1 && interior {
                        assert!(delta < 0.002, "rim leaked onto ID {}", ids[index]);
                    }
                    if ids[index] == 1 && interior {
                        energy += delta;
                        if normals[index][2] < 0.55 {
                            edge.0 += delta;
                            edge.1 += 1;
                        }
                        if normals[index][2] > 0.95 {
                            center.0 += delta;
                            center.1 += 1;
                        }
                    }
                }
                assert!(edge.1 > 10 && center.1 > 10);
                assert!(edge.0 / edge.1 as f32 > center.0 / center.1 as f32 + 0.02);
                energies.push(energy);
            }
            assert!(
                energies[1] > 1. && energies[1] < energies[0] * 0.8,
                "falloff did not concentrate the rim: {energies:?}"
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn retained_rim_pass_follows_camera_and_zero_strength_is_neutral() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let rim = RimLight::new(renderer.context().clone())?;
    let pass = rim.pass()?;
    let plane = Mesh::plane();
    let scene = |yaw, pass: Option<MeshPass>| {
        Scene::new()
            .camera(Camera::orbit(yaw, 0., 3.))
            .object(Object::new(
                plane.clone(),
                Material::color(rgb(0x304354)).unlit(true).mesh_passes(pass),
            ))
    };
    let mut energies = Vec::new();
    for yaw in [0., 1.1] {
        let baseline = read(&mut renderer, &scene(yaw, None), 4)?;
        let lit = read(&mut renderer, &scene(yaw, Some(pass.clone())), 4)?;
        let energy: f32 = baseline
            .linear_rgba
            .as_ref()
            .unwrap()
            .iter()
            .zip(lit.linear_rgba.as_ref().unwrap())
            .map(|(a, b)| difference(a, b))
            .sum();
        energies.push(energy);
        for disabled in [
            rim.clone().intensity(0.),
            rim.clone().color(gpui::rgba(0xffffff00)),
        ] {
            let unlit = read(&mut renderer, &scene(yaw, Some(disabled.pass()?)), 4)?;
            assert_eq!(baseline.rgba, unlit.rgba);
            assert_eq!(baseline.object_ids, unlit.object_ids);
        }
    }
    assert!(
        energies[1] > 5. && energies[1] > energies[0] * 10.,
        "retained rim did not follow the camera: {energies:?}"
    );
    for invalid in [
        rim.clone().intensity(-1.),
        rim.clone().intensity(f32::INFINITY),
        rim.clone().falloff(0.),
        rim.falloff(f32::NAN),
    ] {
        assert!(invalid.pass().is_err());
    }
    Ok(())
}
