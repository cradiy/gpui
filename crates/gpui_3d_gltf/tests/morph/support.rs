use super::*;
use gpui_3d::Scene3dDeviceCapabilities;
use gpui_wgpu::wgpu;

fn capabilities() -> Scene3dDeviceCapabilities {
    Scene3dDeviceCapabilities {
        adapter_info: wgpu::AdapterInfo {
            name: String::new(),
            vendor: 0,
            device: 0,
            device_type: wgpu::DeviceType::Other,
            device_pci_bus_id: String::new(),
            driver: String::new(),
            driver_info: String::new(),
            backend: wgpu::Backend::Vulkan,
            subgroup_min_size: 0,
            subgroup_max_size: 0,
            transient_saves_memory: None,
            limit_bucket: None,
        },
        adapter_features: wgpu::Features::SHADER_F64,
        enabled_features: wgpu::Features::empty(),
        adapter_limits: wgpu::Limits::default(),
        limits: wgpu::Limits::default(),
        downlevel: wgpu::DownlevelCapabilities::default(),
        color_atlas_format: wgpu::TextureFormat::Rgba8Unorm,
        formats: Vec::new(),
        max_image_anisotropy: 1,
    }
}

#[test]
fn asset_support_uses_all_direction_sources_and_enabled_features() {
    let mut fixture = Fixture::new();
    let flat = fixture.json["meshes"][0]["primitives"][0].clone();
    fixture.normals();
    let mut caps = capabilities();
    GpuSceneDeformation::check_support(&asset(&fixture), &caps).unwrap();
    let authored = fixture.json["meshes"][0]["primitives"][0].clone();
    fixture.json["meshes"][0]["primitives"] = json!([authored, flat]);
    let mixed = asset(&fixture);
    assert!(mixed.morphs().iter().all(|m| m.default_weights() == [0.]));
    let error = format!(
        "{:#}",
        GpuSceneDeformation::check_support(&mixed, &caps).unwrap_err()
    );
    assert!(error.contains("node 0 mesh 0 primitive 1"), "{error}");
    assert!(error.contains("flat normal reconstruction"), "{error}");
    assert!(error.contains("enabled SHADER_F64"), "{error}");
    caps.enabled_features = wgpu::Features::SHADER_F64;
    GpuSceneDeformation::check_support(&mixed, &caps).unwrap();
    let generated = asset(&generated_fixture(true));
    GpuSceneDeformation::check_support(&generated, &caps).unwrap();
    caps.enabled_features = wgpu::Features::empty();
    let error = format!(
        "{:#}",
        GpuSceneDeformation::check_support(&generated, &caps).unwrap_err()
    );
    assert!(error.contains("tangent reconstruction"), "{error}");
}

#[test]
fn asset_support_distinguishes_static_morph_and_skin_requirements() {
    let mut fixture = mixed_fixture();
    for mesh in fixture.json["meshes"].as_array_mut().unwrap() {
        for primitive in mesh["primitives"].as_array_mut().unwrap() {
            primitive.as_object_mut().unwrap().remove("targets");
        }
        mesh.as_object_mut().unwrap().remove("weights");
    }
    for node in fixture.json["nodes"].as_array_mut().unwrap() {
        node.as_object_mut().unwrap().remove("weights");
    }
    let skin = asset(&fixture);
    let mut caps = capabilities();
    assert!(skin.morphs().is_empty());
    GpuSceneDeformation::check_support(&skin, &caps).unwrap();
    caps.limits.max_compute_workgroup_size_x = 63;
    let error = format!(
        "{:#}",
        GpuSceneDeformation::check_support(&skin, &caps).unwrap_err()
    );
    assert!(error.contains("Skin computation"), "{error}");
    assert!(error.contains("device enabled 63"), "{error}");

    let mut plain = Fixture::new();
    plain.normals();
    let error = format!(
        "{:#}",
        GpuSceneDeformation::check_support(&asset(&plain), &caps).unwrap_err()
    );
    assert!(error.contains("Morph computation"), "{error}");
    plain.json["meshes"][0]["primitives"][0]
        .as_object_mut()
        .unwrap()
        .remove("targets");
    caps.downlevel
        .flags
        .remove(wgpu::DownlevelFlags::COMPUTE_SHADERS);
    GpuSceneDeformation::check_support(&asset(&plain), &caps).unwrap();
}

#[test]
fn asset_support_checks_tangent_workgroup_storage_without_requiring_indirect_draws() {
    let generated = asset(&generated_fixture(true));
    let mut caps = capabilities();
    caps.enabled_features = wgpu::Features::SHADER_F64;
    caps.downlevel
        .flags
        .remove(wgpu::DownlevelFlags::INDIRECT_EXECUTION);
    GpuSceneDeformation::check_support(&generated, &caps).unwrap();
    caps.limits.max_compute_workgroup_storage_size = 0;
    let error = format!(
        "{:#}",
        GpuSceneDeformation::check_support(&generated, &caps).unwrap_err()
    );
    assert!(error.contains("tangent reconstruction"), "{error}");
    assert!(
        error.contains("max_compute_workgroup_storage_size"),
        "{error}"
    );
}
