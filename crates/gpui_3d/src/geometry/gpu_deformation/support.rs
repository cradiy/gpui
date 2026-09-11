use anyhow::{Result, ensure};
use gpui_wgpu::{Scene3dDeviceCapabilities, wgpu};

pub(in crate::geometry) fn validate(
    capabilities: &Scene3dDeviceCapabilities,
    storage_buffers: u32,
    uniform_buffers: u32,
    workgroup_bytes: u32,
) -> Result<()> {
    validate_limits(
        &capabilities.limits,
        &capabilities.adapter_limits,
        capabilities.downlevel.flags,
        storage_buffers,
        uniform_buffers,
        workgroup_bytes,
    )
}

fn validate_limits(
    limits: &wgpu::Limits,
    adapter: &wgpu::Limits,
    flags: wgpu::DownlevelFlags,
    storage_buffers: u32,
    uniform_buffers: u32,
    workgroup_bytes: u32,
) -> Result<()> {
    ensure!(
        flags.contains(wgpu::DownlevelFlags::COMPUTE_SHADERS),
        "GPU deformation requires COMPUTE_SHADERS"
    );
    macro_rules! require {
        ($field:ident, $minimum:expr) => {
            ensure!(
                limits.$field >= $minimum,
                "GPU deformation requires {} >= {}; device enabled {}, adapter supports {}",
                stringify!($field),
                $minimum,
                limits.$field,
                adapter.$field
            );
        };
    }
    require!(max_storage_buffers_per_shader_stage, storage_buffers);
    require!(max_uniform_buffers_per_shader_stage, uniform_buffers);
    require!(max_bind_groups, 1);
    require!(
        max_bindings_per_bind_group,
        storage_buffers + uniform_buffers
    );
    require!(
        max_buffers_and_acceleration_structures_per_shader_stage,
        storage_buffers + uniform_buffers
    );
    require!(max_storage_buffer_binding_size, 64);
    require!(max_buffer_size, 64);
    if uniform_buffers > 0 {
        require!(max_uniform_buffer_binding_size, 16);
    }
    require!(max_compute_invocations_per_workgroup, 64);
    require!(max_compute_workgroup_size_x, 64);
    require!(max_compute_workgroup_size_y, 1);
    require!(max_compute_workgroup_size_z, 1);
    require!(max_compute_workgroups_per_dimension, 1);
    require!(max_compute_workgroup_storage_size, workgroup_bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_uses_enabled_limits_and_distinguishes_pipeline_requirements() {
        let adapter = wgpu::Limits::default();
        let mut device = wgpu::Limits::downlevel_defaults();
        let flags = wgpu::DownlevelFlags::all();
        validate_limits(&device, &adapter, flags, 4, 1, 0).unwrap();
        device.max_storage_buffers_per_shader_stage = 2;
        validate_limits(&device, &adapter, flags, 2, 0, 2304).unwrap();
        let error = validate_limits(&device, &adapter, flags, 4, 1, 0)
            .unwrap_err()
            .to_string();
        assert!(error.contains("max_storage_buffers_per_shader_stage >= 4"));
        assert!(error.contains("device enabled 2"));
        assert!(error.contains(&format!(
            "adapter supports {}",
            adapter.max_storage_buffers_per_shader_stage
        )));
        device.max_compute_workgroup_storage_size = 2303;
        assert!(validate_limits(&device, &adapter, flags, 2, 0, 2304).is_err());
        device = wgpu::Limits::downlevel_defaults();
        device.max_uniform_buffers_per_shader_stage = 0;
        assert!(validate_limits(&device, &adapter, flags, 4, 1, 0).is_err());
        validate_limits(&device, &adapter, flags, 2, 0, 2304).unwrap();
        assert!(
            validate_limits(
                &device,
                &adapter,
                flags - wgpu::DownlevelFlags::COMPUTE_SHADERS,
                2,
                0,
                2304
            )
            .is_err()
        );
        device.max_compute_workgroup_size_x = 63;
        assert!(validate_limits(&device, &adapter, flags, 2, 0, 2304).is_err());
    }
}
