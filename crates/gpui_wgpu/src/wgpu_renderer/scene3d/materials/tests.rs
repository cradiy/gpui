use super::*;
use crate::wgpu_renderer::scene3d::tests::{frame, object};
use crate::{
    Scene3dChannels, Scene3dMaterialBindingLimits, Scene3dMaterialProgram, Scene3dMaterialValue,
    Scene3dOutputConfig, Scene3dPixels, WgpuContext, WgpuScene3dRenderer,
};
use std::sync::Arc;

const MATERIAL: &str = r#"
struct Controls { color: vec4<f32>, crop: vec4<f32> }
@group(1) @binding(0) var<uniform> controls: Controls;
fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
    return vec4<f32>(controls.color.rgb, select(0.0, 1.0, input.world.x < controls.crop.x));
}
fn material_shading(base: vec3<f32>, input: SurfaceInput,
    gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
    return base;
}
"#;

#[test]
fn scene3d_custom_surface_program_is_valid_and_requires_typed_snapshots() {
    let program = Scene3dMaterialProgram::compile(MATERIAL).unwrap();
    assert!(program.resources()[0].coverage);
    let mut object = object();
    object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(())));
    assert!(snapshot(&object).is_err());
}

#[test]
#[ignore = "requires a GPU adapter"]
fn scene3d_custom_material_preserves_coverage_and_retained_uniforms() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source =
        Scene3dMaterialSource::new(context.clone(), Scene3dMaterialProgram::compile(MATERIAL)?)?;
    let limits = Scene3dMaterialBindingLimits::default();
    let value = |red, blue, crop| {
        Scene3dMaterialValue::Uniform(
            bytemuck::cast_slice(&[red, 0.0_f32, blue, 1.0, crop, 0.0, 0.0, 0.0]).into(),
        )
    };
    let original = source.bind([(0, value(1.0, 0.0, 0.3))], limits)?;
    let changed = original.with_values([(0, value(0.0, 1.0, 0.8))], limits)?;
    let mut object = object();
    object.model[3][2] = 0.5;
    object.alpha_mode = gpui::AlphaMode3d::Mask;
    object.alpha_cutoff = 0.5;
    object.custom_material = Some(gpui::MeshMaterial3d::new(Arc::new(original.clone())));
    let mut input = frame(&[object]);
    input.world_to_view[2][2] = -1.0;
    let retained = input.clone();
    Arc::make_mut(&mut input.objects)[0].custom_material =
        Some(gpui::MeshMaterial3d::new(Arc::new(changed)));

    let mut cache = MaterialCache::default();
    cache.prepare(
        &context.device,
        &[&retained],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
    )?;
    let pipeline = cache.get(&original).mesh.clone();
    cache.prepare(
        &context.device,
        &[&input],
        wgpu::TextureFormat::Rgba8Unorm,
        1,
    )?;
    assert_eq!(cache.get(&original).mesh, pipeline);
    let foreign = WgpuContext::new_headless()?;
    assert!(
        cache
            .prepare(
                &foreign.device,
                &[&input],
                wgpu::TextureFormat::Rgba8Unorm,
                1
            )
            .is_err()
    );

    let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
    for samples in [1, 4] {
        let config = Scene3dOutputConfig {
            size: [64, 64],
            channels: Scene3dChannels::all(),
            color_samples: samples,
        };
        let old = renderer.render(&retained, config)?;
        let new = renderer.render(&input, config)?;
        for (output, changed) in [(old, false), (new, true)] {
            let mut pending = output.readback()?;
            context.device.poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(std::time::Duration::from_secs(10)),
            })?;
            let pixels = pending.try_read()?.context("material readback not ready")?;
            assert_pixel(&pixels, 36, true, changed);
            assert_pixel(&pixels, 48, changed, changed);
        }
    }
    cache.prepare(&context.device, &[], wgpu::TextureFormat::Rgba8Unorm, 1)?;
    assert!(cache.0.is_empty());
    Ok(())
}

fn assert_pixel(pixels: &Scene3dPixels, x: usize, covered: bool, blue: bool) {
    let index = 24 * 64 + x;
    assert_eq!(
        pixels.object_ids.as_ref().unwrap()[index],
        u32::from(covered)
    );
    let rgba = pixels.linear_rgba.as_ref().unwrap()[index];
    let expected = if !covered {
        [0.0; 4]
    } else if blue {
        [0.0, 0.0, 1.0, 1.0]
    } else {
        [1.0, 0.0, 0.0, 1.0]
    };
    for (actual, expected) in rgba.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.001);
    }
    let normal = pixels.world_normals.as_ref().unwrap()[index];
    assert_eq!(normal[3], f32::from(covered));
    if covered {
        assert!((pixels.linear_depth.as_ref().unwrap()[index] - 0.5).abs() < 0.001);
        assert_eq!(&normal[..3], &[0.0, 0.0, 1.0]);
    }
}
