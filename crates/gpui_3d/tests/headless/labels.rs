use std::cell::Cell;

use gpui_3d::{
    HeadlessRenderer, IdRemapConfig, Material, Mesh, Object, Scene, Scene3dChannels,
    Scene3dOutputConfig, WgpuIdRemapper,
};
use gpui_wgpu::{WgpuContext, wgpu};

#[test]
#[ignore = "requires a GPU adapter"]
fn label_remapping_rejects_foreign_frames_before_assignment() -> anyhow::Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let context = renderer.context().clone();
    let foreign = WgpuContext::new_headless()?;
    let mapper = WgpuIdRemapper::new(context.clone(), IdRemapConfig::default())?;
    let foreign_mapper = WgpuIdRemapper::new(foreign.clone(), IdRemapConfig::default())?;
    let scene = Scene::new().object(Object::new(Mesh::cube(), Material::color(gpui::white())));
    let frame = renderer.render(
        &scene,
        Scene3dOutputConfig {
            size: [8, 8],
            channels: Scene3dChannels::all(),
            color_samples: 1,
        },
    )?;
    for texture in [
        frame.gpu().color(),
        frame.gpu().linear_color(),
        frame.gpu().object_ids(),
        frame.gpu().linear_depth(),
        frame.gpu().world_normals(),
    ] {
        let texture = texture.expect("requested channel missing");
        texture.check_device(&context.device)?;
        assert!(texture.check_device(&foreign.device).is_err());
    }
    let input = frame.gpu().object_ids().unwrap();
    mapper.validate_input(input, 1)?;
    let mut encoder = foreign
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    for error in [
        foreign_mapper.validate_input(input, 1).unwrap_err(),
        foreign_mapper.render(input, &[7]).unwrap_err(),
        foreign_mapper
            .encode(&mut encoder, input, &[7])
            .unwrap_err(),
    ] {
        assert!(error.to_string().contains("different device"));
    }
    let assignments = Cell::new(0);
    let error = frame
        .label_texture(&foreign_mapper, |_| {
            assignments.set(assignments.get() + 1);
            7
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("different device"));
    let error = frame
        .encode_label_texture(&foreign_mapper, &mut encoder, |_| {
            assignments.set(assignments.get() + 1);
            7
        })
        .err()
        .unwrap();
    assert!(error.to_string().contains("different device"));
    assert_eq!(assignments.get(), 0);
    drop(encoder);

    let labels = frame.label_texture(&mapper, |_| {
        assignments.set(assignments.get() + 1);
        7
    })?;
    assert_eq!(assignments.get(), 1);
    assert_eq!(labels.frame_id(), frame.frame_id());
    assert_eq!(labels.label_for_object(1), Some(7));
    drop(frame);
    drop(renderer);
    labels.texture().check_device(&context.device)?;
    assert!(labels.texture().check_device(&foreign.device).is_err());
    mapper.validate_input(labels.texture(), 7)?;
    Ok(())
}
