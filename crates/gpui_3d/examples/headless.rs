use anyhow::{Result, ensure};
use gpui::{rgb, rgba};
use gpui_3d::{
    AlphaMode, Camera, HeadlessRenderer, Material, Mesh, Object, Scene, Scene3dChannels,
    Scene3dOutputConfig,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

fn main() -> Result<()> {
    let directory = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("render-output"));
    let scene = Scene::new()
        .camera(Camera::orbit(0.35, 0.25, 6.))
        .object(
            Object::new(Mesh::cube(), Material::color(rgb(0xe4b183)))
                .id("warm cube")
                .position([-0.7, 0., 0.2])
                .rotation([0.2, 0.4, 0.]),
        )
        .object(
            Object::new(Mesh::cube(), Material::color(rgb(0x71c2d9)))
                .id("cool cube")
                .position([0.7, -0.2, -0.5])
                .rotation([0., -0.4, 0.2]),
        )
        .object(
            Object::new(
                Mesh::plane(),
                Material::color(rgba(0xc1a0f760)).alpha_mode(AlphaMode::Blend),
            )
            .position([0.4, 0.1, 1.])
            .scale([0.7, 1.8, 1.]),
        )
        .object(
            Object::new(Mesh::plane(), Material::color(rgb(0x506475)))
                .position([0., -1.1, 0.])
                .rotation([-std::f32::consts::FRAC_PI_2, 0., 0.])
                .scale([4., 4., 1.]),
        );
    let mut renderer = HeadlessRenderer::new()?;
    let frame = renderer.render(
        &scene,
        Scene3dOutputConfig {
            size: [800, 600],
            channels: Scene3dChannels::all(),
            color_samples: 1,
        },
    )?;
    let mut readback = frame.readback()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut result = loop {
        if let Some(result) = readback.try_read()? {
            break result;
        }
        ensure!(Instant::now() < deadline, "GPU readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    };
    let ids = result.pixels.object_ids.as_ref().unwrap();
    for object in result.objects() {
        let count = ids.iter().filter(|id| **id == object.output_id).count();
        println!(
            "ID {}: {:?}, {} visible pixels",
            object.output_id, object.id, count
        );
    }
    let id_preview = image::RgbaImage::from_fn(800, 600, |x, y| {
        let id = ids[(y * 800 + x) as usize];
        image::Rgba([
            id.wrapping_mul(71) as u8,
            id.wrapping_mul(137) as u8,
            id.wrapping_mul(213) as u8,
            if id == 0 { 0 } else { 255 },
        ])
    });
    let depth = result.pixels.linear_depth.as_ref().unwrap();
    let normals = result.pixels.world_normals.as_ref().unwrap();
    let (near, far) = depth
        .iter()
        .zip(normals)
        .filter(|(_, n)| n[3] > 0.)
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), (&z, _)| {
            (min.min(z), max.max(z))
        });
    let depth_preview = image::RgbaImage::from_fn(800, 600, |x, y| {
        let index = (y * 800 + x) as usize;
        let value = ((1. - (depth[index] - near) / (far - near).max(0.001)).clamp(0., 1.) * 255.)
            .round() as u8;
        image::Rgba([
            value,
            value,
            value,
            if normals[index][3] > 0. { 255 } else { 0 },
        ])
    });
    let normal_preview = image::RgbaImage::from_fn(800, 600, |x, y| {
        let n = normals[(y * 800 + x) as usize];
        image::Rgba([
            ((n[0] * 0.5 + 0.5) * 255.).round() as u8,
            ((n[1] * 0.5 + 0.5) * 255.).round() as u8,
            ((n[2] * 0.5 + 0.5) * 255.).round() as u8,
            if n[3] > 0. { 255 } else { 0 },
        ])
    });
    let mut color = result.pixels.rgba.take().unwrap();
    for pixel in color.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    std::fs::create_dir_all(&directory)?;
    image::save_buffer(
        directory.join("color.png"),
        &color,
        800,
        600,
        image::ColorType::Rgba8,
    )?;
    id_preview.save(directory.join("ids.png"))?;
    depth_preview.save(directory.join("depth.png"))?;
    normal_preview.save(directory.join("normals.png"))?;
    println!(
        "Saved color, ID, depth, and normal previews to {}",
        directory.display()
    );
    println!("Depth range: {near:.3}–{far:.3} scene units; near is brighter");
    Ok(())
}
