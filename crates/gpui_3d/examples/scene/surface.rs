use gpui::{RenderImage, rgb};
use gpui_3d::{Material, MaterialTexture, PbrMaterial};
use std::sync::Arc;

pub(super) fn normal_image() -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_fn(128, 128, |x, y| {
            let u = x as f32 / 128. * std::f32::consts::TAU;
            let v = y as f32 / 128. * std::f32::consts::TAU;
            let normal = [-0.8 * (3. * u).cos(), -0.35 * (2. * v).cos(), 1.];
            let length = normal.iter().map(|value| value * value).sum::<f32>().sqrt();
            let encoded = normal.map(|value| ((value / length * 0.5 + 0.5) * 255.).round() as u8);
            image::Rgba([encoded[0], encoded[1], encoded[2], 255])
        }),
    )]))
}

pub(super) fn material(tinted: bool, normal: Option<&Arc<RenderImage>>) -> Material {
    let material =
        Material::color(rgb(if tinted { 0xf09e8e } else { 0x8dd8e8 })).pbr(PbrMaterial {
            metallic: 0.,
            roughness: 0.35,
            emissive: [0.; 3],
        });
    match normal {
        Some(image) => material.normal_texture(MaterialTexture::new(image.clone())),
        None => material,
    }
}
