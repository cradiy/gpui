use super::*;
use crate::{Material, Mesh, Object, ObjectUpdate};
use gpui_wgpu::wgpu;

#[test]
#[ignore = "requires a GPU adapter"]
fn repeated_and_updated_outputs_keep_distinct_readback_provenance() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut renderer = HeadlessRenderer::with_context(context.clone())?;
    let scene = Scene::new().object(Object::new(
        Mesh::cube(),
        Material::color(gpui::rgb(0xffffff)),
    ));
    let config = Scene3dOutputConfig {
        size: [32, 32],
        channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
        color_samples: 1,
    };
    let original = renderer.render(&scene, config)?;
    let identity = original.frame_id().clone();
    let mut pick = original.pick([16, 16])?;
    assert_eq!(pick.frame_id(), &identity);
    let repeated = renderer.render(&scene, config)?;
    assert_ne!(repeated.frame_id(), &identity);
    let changed = scene.with_object_updates([(
        1,
        ObjectUpdate::new()
            .material(Material::color(gpui::rgba(0xffffff00)).alpha_mode(crate::AlphaMode::Mask)),
    )])?;
    let updated = renderer.render(&changed, config)?;
    assert_ne!(updated.frame_id(), repeated.frame_id());
    assert_ne!(updated.frame_id(), &identity);
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(10)),
    })?;
    let result = pick.try_read()?.expect("pick mapping should complete");
    assert_eq!(result.frame_id(), &identity);
    drop(pick);
    let region = Scene3dReadbackRegion {
        origin: [12, 8],
        size: [8, 4],
    };
    let mut readback =
        original.readback_region(region, Scene3dReadbackConfig::new(config.channels))?;
    assert_eq!(readback.layout().region, region);
    assert_eq!(readback.layout().output_size, [32, 32]);
    assert_eq!(readback.frame_id(), &identity);
    drop(original);
    drop(renderer);
    context.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(10)),
    })?;
    let pixels = readback.try_read()?.expect("frame mapping should complete");
    assert_eq!(pixels.pixels.size, [8, 4]);
    assert_eq!(pixels.layout().region, region);
    assert_eq!(pixels.coverage()?.layout().output_size, [32, 32]);
    assert_eq!(pixels.frame_id(), &identity);
    assert_eq!(pixels.coverage()?.frame_id(), &identity);
    assert_eq!(
        pixels
            .label_image(1024, |object| object.output_id)?
            .frame_id(),
        &identity
    );
    Ok(())
}
