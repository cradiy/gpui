use anyhow::{Context as _, Result};
use gpui::Window;
use gpui_3d::{
    Scene3dMaterialBindingLimits, Scene3dMaterialProgram, Scene3dMaterialSnapshot,
    Scene3dMaterialSource, Scene3dMaterialValue, WgpuContext,
};
use gpui_wgpu::wgpu;
use std::sync::Arc;

pub(super) struct Programs {
    context: WgpuContext,
    pub toon: Scene3dMaterialSnapshot,
    pub sphere: Scene3dMaterialSnapshot,
    pub outline: Scene3dMaterialSnapshot,
}

fn uniform(settings: [f32; 4]) -> Scene3dMaterialValue {
    Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&settings).into())
}

impl Programs {
    pub fn matches_window(&self, window: &Window) -> bool {
        !self.context.device_lost()
            && WgpuContext::for_window(window)
                .is_some_and(|context| Arc::ptr_eq(&context.device, &self.context.device))
    }

    pub fn new(window: &Window, bands: f32, brightness: f32, mesh: &gpui_3d::Mesh) -> Result<Self> {
        let context = WgpuContext::for_window(window).context("A wgpu window is required")?;
        let limits = Scene3dMaterialBindingLimits::default();
        let outline_source = Scene3dMaterialSource::new(
            context.clone(),
            Scene3dMaterialProgram::compile_with_attributes(
                include_str!("outline.wgsl"),
                &[gpui_3d::Scene3dVertexAttribute::new(
                    "width",
                    wgpu::VertexFormat::Float32,
                )],
            )?,
        )?;
        let weights: Vec<_> = mesh
            .vertices()
            .iter()
            .map(|vertex| 0.2 + 0.8 * (vertex.normal[1] * 0.5 + 0.5).clamp(0., 1.))
            .collect();
        let streams = outline_source.bind_vertex_streams(
            mesh.vertex_count(),
            &[(
                "width",
                gpui_3d::Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&weights)),
            )],
            1024 * 1024,
        )?;
        let outline = outline_source
            .bind([], limits)?
            .with_vertex_streams(streams)?;
        let toon = Scene3dMaterialSource::new(
            context.clone(),
            Scene3dMaterialProgram::compile(include_str!("toon.wgsl"))?,
        )?
        .bind([(0, uniform([bands, 0.2, 0.0, 0.0]))], limits)?;
        let source = Scene3dMaterialSource::new(
            context.clone(),
            Scene3dMaterialProgram::compile(include_str!("sphere_map.wgsl"))?,
        )?;
        let pixels = image::RgbaImage::from_fn(128, 128, |x, y| {
            let u = x as f32 / 127.0;
            let v = y as f32 / 127.0;
            let stripe = (-((v - 0.3) / 0.065).powi(2)).exp();
            let warm = (-(((u - 0.72) / 0.27).powi(2) + ((v - 0.58) / 0.23).powi(2))).exp();
            let color = [
                0.08 + 0.85 * warm + 0.65 * stripe,
                0.14 + 0.38 * warm + 0.8 * stripe,
                0.28 + 0.3 * u + 0.7 * stripe,
            ]
            .map(|value| (value.min(1.0) * 255.0).round() as u8);
            image::Rgba([color[0], color[1], color[2], 255])
        });
        let texture = context.create_texture(&wgpu::TextureDescriptor {
            label: Some("sphere map"),
            size: wgpu::Extent3d {
                width: 128,
                height: 128,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        context.queue.write_texture(
            texture.as_image_copy(),
            pixels.as_raw(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(128 * 4),
                rows_per_image: None,
            },
            texture.size(),
        );
        let sampler = context.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let sphere = source.bind(
            [
                (0, uniform([brightness, 0.0, 0.0, 0.0])),
                (
                    1,
                    Scene3dMaterialValue::Texture(texture.create_view(&Default::default())),
                ),
                (2, Scene3dMaterialValue::Sampler(sampler)),
            ],
            limits,
        )?;
        Ok(Self {
            outline,
            context,
            toon,
            sphere,
        })
    }

    pub fn update(&mut self, bands: f32, brightness: f32) -> Result<()> {
        let limits = Scene3dMaterialBindingLimits::default();
        let toon = self
            .toon
            .with_values([(0, uniform([bands, 0.2, 0.0, 0.0]))], limits)?;
        let sphere = self
            .sphere
            .with_values([(0, uniform([brightness, 0.0, 0.0, 0.0]))], limits)?;
        self.toon = toon;
        self.sphere = sphere;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_materials_validate_with_shading_only_resources() {
        Scene3dMaterialProgram::compile_with_attributes(
            include_str!("outline.wgsl"),
            &[gpui_3d::Scene3dVertexAttribute::new(
                "width",
                wgpu::VertexFormat::Float32,
            )],
        )
        .unwrap();
        for source in [include_str!("toon.wgsl"), include_str!("sphere_map.wgsl")] {
            let program = Scene3dMaterialProgram::compile(source).unwrap();
            assert!(
                program
                    .resources()
                    .iter()
                    .all(|resource| resource.shading && !resource.coverage)
            );
            let controls = &program.resources()[0];
            assert_eq!(
                controls.kind,
                gpui_3d::Scene3dMaterialResourceKind::Uniform { min_size: 16 }
            );
        }
    }
}
