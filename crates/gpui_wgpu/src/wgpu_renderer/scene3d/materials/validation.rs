use anyhow::{Result, ensure};

pub(crate) fn validate_settings(object: &gpui::MeshDraw3d) -> Result<()> {
    #[cfg(target_family = "wasm")]
    ensure!(
        object.custom_material.is_none() && object.mesh_passes.is_empty(),
        "custom 3D materials require a native renderer"
    );
    #[cfg(not(target_family = "wasm"))]
    {
        ensure!(
            object.mesh_passes.len() <= gpui::MAX_MESH_PASSES_3D,
            "too many additional mesh passes"
        );
        if let Some(snapshot) = super::snapshot(object)? {
            snapshot.validate_vertex_count(object.mesh.vertices().len())?;
        }
        for pass in object.mesh_passes.iter() {
            ensure!(pass.state.is_valid(), "invalid additional mesh pass state");
            let snapshot = super::pass_snapshot(pass)?;
            snapshot.validate_vertex_count(object.mesh.vertices().len())?;
            super::mesh_pass::Expansion::new(
                snapshot.source().program().vertex_attributes(),
                pass.expansion.as_ref(),
            )?;
        }
    }
    Ok(())
}

#[cfg(not(target_family = "wasm"))]
pub(crate) fn validate_devices(frame: &gpui::Scene3dFrame, device: &wgpu::Device) -> Result<()> {
    for object in frame.objects.iter() {
        if let Some(snapshot) = super::snapshot(object)? {
            let context = snapshot.source().context();
            ensure!(
                !context.device_lost() && std::ptr::eq(device, context.device.as_ref()),
                "3D material belongs to a different or lost device"
            );
        }
        for pass in object.mesh_passes.iter() {
            let context = super::pass_snapshot(pass)?.source().context();
            ensure!(
                !context.device_lost() && std::ptr::eq(device, context.device.as_ref()),
                "3D mesh pass belongs to a different or lost device"
            );
        }
    }
    Ok(())
}
