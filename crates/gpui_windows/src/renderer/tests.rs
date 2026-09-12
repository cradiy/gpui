use super::*;
use gpui_wgpu::{WgpuContext, wgpu};
use std::{cell::RefCell, rc::Rc};
use windows::{
    Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_OVERLAPPEDWINDOW,
    },
    core::w,
};

struct TestWindow(HWND);

impl TestWindow {
    fn new() -> Result<Self> {
        Ok(Self(unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("GPUI renderer test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                128,
                128,
                None,
                None,
                None,
                None,
            )?
        }))
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.0).unwrap() };
    }
}

fn context(renderer: &WindowsRenderer) -> Arc<WgpuContext> {
    renderer
        .sprite_atlas()
        .renderer_context()
        .unwrap()
        .downcast()
        .unwrap()
}

#[test]
#[ignore = "requires a Windows desktop and a DX12 adapter"]
fn native_windows_share_dx12_and_render_after_resize_and_transparency_changes() -> Result<()> {
    let devices = DirectXDevices::new()?;
    let first = TestWindow::new()?;
    let second = TestWindow::new()?;
    let shared = Rc::new(RefCell::new(None));
    let mut renderer = WindowsRenderer::new(first.0, &devices, false, shared.clone())?;
    let mut other = WindowsRenderer::new(second.0, &devices, false, shared)?;
    assert!(renderer.wgpu().unwrap().scene3d_support().is_supported());
    let gpu = context(&renderer);
    assert_eq!(gpu.adapter.get_info().backend, wgpu::Backend::Dx12);
    assert!(Arc::ptr_eq(&gpu.device, &context(&other).device));
    let validation = gpu.device.push_error_scope(wgpu::ErrorFilter::Validation);

    let mut scene = Scene::default();
    scene.finish();
    for size in [
        gpui::size(DevicePixels(64), DevicePixels(48)),
        gpui::size(DevicePixels(96), DevicePixels(80)),
    ] {
        renderer.resize(size)?;
        assert_eq!(renderer.wgpu().unwrap().viewport_size(), size);
        for background in [
            WindowBackgroundAppearance::Opaque,
            WindowBackgroundAppearance::Transparent,
            WindowBackgroundAppearance::MicaBackdrop,
        ] {
            renderer.draw(&scene, background)?;
            assert!(!renderer.prepare_frame()?);
        }
    }
    renderer
        .wgpu_mut()
        .unwrap()
        .set_scene3d_output_cache_budget(1024);
    renderer.wgpu_mut().unwrap().clear_scene3d_caches();
    other.draw(&scene, WindowBackgroundAppearance::Transparent)?;
    assert!(!other.prepare_frame()?);
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    assert!(gpui::block_on(validation.pop()).is_none());
    renderer.destroy();
    other.destroy();
    Ok(())
}

#[test]
#[ignore = "requires a Windows desktop and a D3D11 adapter"]
fn disabled_direct_composition_uses_d3d11() -> Result<()> {
    let devices = DirectXDevices::new()?;
    let window = TestWindow::new()?;
    let mut renderer = WindowsRenderer::new(window.0, &devices, true, Rc::new(RefCell::new(None)))?;
    assert!(renderer.wgpu().is_none());
    assert!(renderer.sprite_atlas().renderer_context().is_none());
    renderer.resize(gpui::size(DevicePixels(64), DevicePixels(64)))?;
    let mut scene = Scene::default();
    scene.finish();
    renderer.draw(&scene, WindowBackgroundAppearance::Opaque)?;
    assert!(!renderer.prepare_frame()?);
    Ok(())
}
