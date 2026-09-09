use anyhow::{Result, ensure};
use gpui::rgb;
use gpui_3d::{
    AffineTransform, Camera, HeadlessRenderer, Material, Mesh, Node, Scene3dOutputConfig,
    SceneGraph,
};
use std::time::{Duration, Instant};

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .unwrap_or_else(|| "scene.png".into());
    let mut graph = SceneGraph::new();
    let cube = Mesh::cube();
    for (id, position, color) in [
        ("coral", [-0.8, 0., 0.4], 0xf09e8e),
        ("ice", [0.8, 0., -0.4], 0x8dd8e8),
        ("gold", [0., -1., -0.8], 0xf4cf89),
    ] {
        graph.insert(
            None,
            Node::new()
                .id(id)
                .mesh(cube.clone(), Material::color(rgb(color)))
                .transform(AffineTransform::from_translation(position)?),
        )?;
    }
    let evaluated = graph.evaluate()?;
    let camera =
        Camera::orbit(0.5, 0.35, 8.).frame_bounds(evaluated.bounds().unwrap(), 4. / 3., 1.3)?;
    let mut renderer = HeadlessRenderer::new()?;
    let frame = renderer.render(
        &evaluated.scene(camera),
        Scene3dOutputConfig::new([800, 600]),
    )?;
    let mut readback = frame.readback()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut pixels = loop {
        if let Some(frame) = readback.try_read()? {
            break frame;
        }
        ensure!(Instant::now() < deadline, "GPU readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    };
    let mut rgba = pixels.pixels.rgba.take().unwrap();
    // PNG stores straight alpha.
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    image::save_buffer(&path, &rgba, 800, 600, image::ColorType::Rgba8)?;
    println!("Saved {}", std::path::Path::new(&path).display());
    let ids = pixels.pixels.object_ids.as_ref().unwrap();
    for object in pixels.objects() {
        let count = ids.iter().filter(|id| **id == object.output_id).count();
        println!(
            "ID {}: {:?}, {} visible pixels",
            object.output_id, object.id, count
        );
    }
    Ok(())
}
