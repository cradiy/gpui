use std::{num::NonZeroIsize, sync::Arc};

use anyhow::Result;
use gpui::{DevicePixels, GpuSpecs, PlatformAtlas, Scene, Size, WindowBackgroundAppearance};
use gpui_wgpu::{GpuContext, WgpuRenderer, WgpuSurfaceConfig};
use raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use windows::Win32::Foundation::HWND;

use crate::{DirectXDevices, DirectXRenderer, SafeHwnd};

pub(crate) enum WindowsRenderer {
    Wgpu {
        renderer: Box<WgpuRenderer>,
        window: SafeHwnd,
        size: Size<DevicePixels>,
        redraw: bool,
    },
    DirectX(Box<DirectXRenderer>),
}

impl HasWindowHandle for SafeHwnd {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.as_raw().0 as isize).ok_or(HandleError::Unavailable)?;
        let handle = raw_window_handle::Win32WindowHandle::new(hwnd);
        // The renderer is destroyed before the owning window releases its HWND.
        Ok(unsafe { WindowHandle::borrow_raw(handle.into()) })
    }
}

impl HasDisplayHandle for SafeHwnd {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        Ok(DisplayHandle::windows())
    }
}

impl WindowsRenderer {
    pub(crate) fn new(
        hwnd: HWND,
        devices: &DirectXDevices,
        disable_direct_composition: bool,
        context: GpuContext,
    ) -> Result<Self> {
        if !disable_direct_composition {
            let window = SafeHwnd::from(hwnd);
            match WgpuRenderer::new(
                context.clone(),
                &window,
                WgpuSurfaceConfig {
                    size: gpui::size(DevicePixels(1), DevicePixels(1)),
                    transparent: false,
                    preferred_present_mode: None,
                },
                None,
            ) {
                Ok(mut renderer) => {
                    renderer.set_subpixel_layout(DirectXRenderer::get_font_info().is_bgr);
                    return Ok(Self::Wgpu {
                        renderer: Box::new(renderer),
                        window,
                        size: gpui::size(DevicePixels(1), DevicePixels(1)),
                        redraw: false,
                    });
                }
                Err(error) => {
                    log::warn!("Windows WGPU initialization failed, using D3D11: {error:#}")
                }
            }
        }
        Ok(Self::DirectX(Box::new(DirectXRenderer::new(
            hwnd,
            devices,
            disable_direct_composition,
        )?)))
    }

    pub(crate) fn wgpu(&self) -> Option<&WgpuRenderer> {
        match self {
            Self::Wgpu { renderer, .. } => Some(renderer),
            Self::DirectX(_) => None,
        }
    }

    pub(crate) fn wgpu_mut(&mut self) -> Option<&mut WgpuRenderer> {
        match self {
            Self::Wgpu { renderer, .. } => Some(renderer),
            Self::DirectX(_) => None,
        }
    }

    pub(crate) fn prepare_frame(&mut self) -> Result<bool> {
        let Self::Wgpu {
            renderer,
            window,
            size,
            redraw,
        } = self
        else {
            return Ok(false);
        };
        if renderer.device_lost() {
            renderer.recover(window)?;
            renderer.set_subpixel_layout(DirectXRenderer::get_font_info().is_bgr);
            renderer.update_drawable_size(*size);
            *redraw = true;
        }
        Ok(renderer.needs_redraw() | std::mem::take(redraw))
    }

    pub(crate) fn draw(
        &mut self,
        scene: &Scene,
        background: WindowBackgroundAppearance,
    ) -> Result<()> {
        match self {
            Self::Wgpu {
                renderer, redraw, ..
            } => {
                if !renderer.device_lost() {
                    renderer.update_transparency(background != WindowBackgroundAppearance::Opaque);
                    *redraw |= !renderer.draw(scene);
                }
                Ok(())
            }
            Self::DirectX(renderer) => renderer.draw(scene, background),
        }
    }

    pub(crate) fn resize(&mut self, size: Size<DevicePixels>) -> Result<()> {
        let size = gpui::size(
            DevicePixels(size.width.0.max(1)),
            DevicePixels(size.height.0.max(1)),
        );
        match self {
            Self::Wgpu {
                renderer,
                size: drawable_size,
                ..
            } => {
                *drawable_size = size;
                if !renderer.device_lost() {
                    renderer.update_drawable_size(size);
                }
                Ok(())
            }
            Self::DirectX(renderer) => renderer.resize(size),
        }
    }

    pub(crate) fn sprite_atlas(&self) -> Arc<dyn PlatformAtlas> {
        match self {
            Self::Wgpu { renderer, .. } => renderer.sprite_atlas().clone(),
            Self::DirectX(renderer) => renderer.sprite_atlas(),
        }
    }

    pub(crate) fn gpu_specs(&self) -> Result<GpuSpecs> {
        match self {
            Self::Wgpu { renderer, .. } => Ok(renderer.gpu_specs()),
            Self::DirectX(renderer) => renderer.gpu_specs(),
        }
    }

    pub(crate) fn handle_device_lost(&mut self, devices: &DirectXDevices) -> Result<()> {
        match self {
            Self::Wgpu { .. } => Ok(()),
            Self::DirectX(renderer) => renderer.handle_device_lost(devices),
        }
    }

    pub(crate) fn mark_drawable(&mut self) {
        if let Self::DirectX(renderer) = self {
            renderer.mark_drawable();
        }
    }

    pub(crate) fn destroy(&mut self) {
        if let Some(renderer) = self.wgpu_mut() {
            renderer.destroy();
        }
    }
}

#[cfg(test)]
mod tests;
