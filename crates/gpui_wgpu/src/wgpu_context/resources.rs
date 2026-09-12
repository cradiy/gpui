use super::WgpuContext;
use std::{ops::Deref, sync::Arc};
use wgpu::util::DeviceExt as _;

/// A resource retaining the device that created it. Construct through `WgpuContext`.
/// Borrowing the raw handle supports application commands without changing ownership.
/// Clones share the GPU allocation; ownership does not imply immutable contents.
#[derive(Clone, Debug)]
pub struct WgpuResource<T> {
    resource: T,
    device: Arc<wgpu::Device>,
}

impl<T> WgpuResource<T> {
    pub fn raw(&self) -> &T {
        &self.resource
    }

    /// Requires the same shared device allocation, as preserved by `WgpuContext::clone`.
    /// Rewrapping a raw device handle in a different `Arc` does not transfer this identity.
    pub fn check_device(&self, device: &Arc<wgpu::Device>) -> anyhow::Result<()> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.device, device),
            "GPU resource belongs to a different device"
        );
        Ok(())
    }
}

impl<T> Deref for WgpuResource<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        self.raw()
    }
}

impl WgpuContext {
    /// Creates a buffer with retained device identity. WGPU validates the descriptor.
    pub fn create_buffer(
        &self,
        descriptor: &wgpu::BufferDescriptor<'_>,
    ) -> WgpuResource<wgpu::Buffer> {
        WgpuResource {
            resource: self.device.create_buffer(descriptor),
            device: self.device.clone(),
        }
    }

    /// Uploads initial contents and retains the creating device.
    pub fn create_buffer_init(
        &self,
        descriptor: &wgpu::util::BufferInitDescriptor<'_>,
    ) -> WgpuResource<wgpu::Buffer> {
        WgpuResource {
            resource: self.device.create_buffer_init(descriptor),
            device: self.device.clone(),
        }
    }

    /// Creates a sampler with retained device identity. WGPU validates the descriptor.
    pub fn create_sampler(
        &self,
        descriptor: &wgpu::SamplerDescriptor<'_>,
    ) -> WgpuResource<wgpu::Sampler> {
        WgpuResource {
            resource: self.device.create_sampler(descriptor),
            device: self.device.clone(),
        }
    }

    /// Creates a texture whose views inherit its creating device.
    pub fn create_texture(
        &self,
        descriptor: &wgpu::TextureDescriptor<'_>,
    ) -> WgpuResource<wgpu::Texture> {
        WgpuResource {
            resource: self.device.create_texture(descriptor),
            device: self.device.clone(),
        }
    }
}

impl WgpuResource<wgpu::Texture> {
    /// Creates a view retaining this texture's device. WGPU validates the descriptor.
    pub fn create_view(
        &self,
        descriptor: &wgpu::TextureViewDescriptor<'_>,
    ) -> WgpuResource<wgpu::TextureView> {
        WgpuResource {
            resource: self.resource.create_view(descriptor),
            device: self.device.clone(),
        }
    }
}
