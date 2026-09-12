use std::time::{Duration, Instant};

use gpui::rgb;
use gpui_3d::{
    Camera, HeadlessRenderer, Material, Mesh, Object, Scene, Scene3dChannels, Scene3dOutputConfig,
    SphereOptions,
};
use gpui_3d_effects::OrbitLight;

#[test]
#[ignore = "requires a GPU adapter"]
fn transparent_stage_and_orbit_depth_occlusion() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let config = Scene3dOutputConfig {
        size: [320, 320],
        channels: Scene3dChannels::all(),
        color_samples: 1,
    };
    let mut read = |scene: &Scene| -> anyhow::Result<_> {
        let frame = renderer.render(scene, config)?;
        let mut pending = frame.readback()?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(output) = pending.try_read()? {
                return Ok(output.pixels);
            }
            anyhow::ensure!(Instant::now() < deadline, "render readback timed out");
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    let light = OrbitLight::default().tilt([1.05, 0.]);
    let camera = Camera::orbit(0., 0., 4.);
    let alone = read(&Scene::new().camera(camera).object(light.object(0.)))?;
    let footprint = |ids: &[u32]| {
        let mut bounds = [320usize, 320, 0, 0];
        for (index, id) in ids.iter().enumerate() {
            if *id != 0 {
                let (x, y) = (index % 320, index / 320);
                bounds = [
                    bounds[0].min(x),
                    bounds[1].min(y),
                    bounds[2].max(x),
                    bounds[3].max(y),
                ];
            }
        }
        bounds
    };
    let bright_center = |pixels: &[[f32; 4]]| {
        let mut sum = [0f64; 3];
        for (index, pixel) in pixels.iter().enumerate() {
            let weight = f64::from((pixel[0].max(pixel[1]).max(pixel[2]) - 0.4).max(0.));
            sum[0] += (index % 320) as f64 * weight;
            sum[1] += (index / 320) as f64 * weight;
            sum[2] += weight;
        }
        assert!(sum[2] > 0.);
        [sum[0] / sum[2], sum[1] / sum[2]]
    };
    let bounds = footprint(alone.object_ids.as_ref().unwrap());
    let initial_center = bright_center(alone.linear_rgba.as_ref().unwrap());
    for phase in [std::f32::consts::FRAC_PI_2, std::f32::consts::PI] {
        let advanced = read(&Scene::new().camera(camera).object(light.object(phase)))?;
        let advanced_bounds = footprint(advanced.object_ids.as_ref().unwrap());
        assert!(
            bounds
                .into_iter()
                .zip(advanced_bounds)
                .all(|(a, b)| a.abs_diff(b) <= 2),
            "orbit footprint changed: {bounds:?} -> {advanced_bounds:?}"
        );
        let center = bright_center(advanced.linear_rgba.as_ref().unwrap());
        assert!(
            (center[0] - initial_center[0]).hypot(center[1] - initial_center[1]) > 20.,
            "highlight did not advance: {initial_center:?} -> {center:?}"
        );
    }
    let combined = read(
        &Scene::new()
            .camera(camera)
            .object(Object::new(
                Mesh::sphere(SphereOptions {
                    radius: 0.82,
                    ..Default::default()
                })?,
                Material::color(rgb(0x182532)),
            ))
            .object(light.object(0.)),
    )?;
    let alone_ids = alone.object_ids.unwrap();
    let combined_ids = combined.object_ids.unwrap();
    let mut front = 0;
    let mut hidden = 0;
    for (a, b) in alone_ids.iter().zip(&combined_ids) {
        if *a == 1 {
            if *b == 1 {
                hidden += 1;
            }
            if *b == 2 {
                front += 1;
            }
        }
    }
    assert!(
        front > 20 && hidden > 20,
        "orbit must pass in front and behind: {front} visible, {hidden} hidden"
    );
    let pixels = combined.linear_rgba.unwrap();
    assert_eq!(pixels[0][3], 0.);
    assert_eq!(pixels[pixels.len() - 1][3], 0.);
    assert!(pixels.iter().all(|p| p.iter().all(|v| v.is_finite())));
    Ok(())
}
