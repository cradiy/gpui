use super::*;

/// One tightly packed custom vertex stream. External resources must be created through
/// the material source's `WgpuContext`, with one record per mesh vertex.
/// Submit GPU producers on that context's queue before binding or updating streams.
/// GPU contents are not read back: callers must ensure finite float lanes and valid
/// vertex ordering, and leave inputs unmapped and undestroyed during GPU use.
#[derive(Clone, Copy, Debug)]
pub enum Scene3dVertexStreamValue<'a> {
    /// Copies CPU bytes into a private storage buffer, validating finite float lanes.
    Bytes(&'a [u8]),
    /// Retains a STORAGE buffer without copying. Do not write, map, or destroy it while
    /// any stream/material snapshot or queued draw may use it.
    SharedBuffer(&'a WgpuResource<wgpu::Buffer>),
    /// Copies a COPY_SRC buffer into private storage without CPU readback. Subsequent
    /// writes submitted on the same queue may reuse the source without changing the
    /// snapshot. Do not map or explicitly destroy the source before the copy completes.
    CopiedBuffer(&'a WgpuResource<wgpu::Buffer>),
}

impl Scene3dVertexStreamValue<'_> {
    pub(super) fn check_device(&self, device: &Arc<wgpu::Device>) -> Result<()> {
        match self {
            Self::Bytes(_) => Ok(()),
            Self::SharedBuffer(buffer) | Self::CopiedBuffer(buffer) => buffer.check_device(device),
        }
    }

    pub(super) fn validate(
        &self,
        name: &str,
        format: wgpu::VertexFormat,
        expected: u64,
    ) -> Result<()> {
        match self {
            Self::Bytes(bytes) => {
                ensure!(
                    bytes.len() as u64 == expected,
                    "custom vertex stream {name} requires exactly {expected} bytes"
                );
                if matches!(
                    format,
                    wgpu::VertexFormat::Float32
                        | wgpu::VertexFormat::Float32x2
                        | wgpu::VertexFormat::Float32x3
                        | wgpu::VertexFormat::Float32x4
                ) {
                    ensure!(
                        bytes.chunks_exact(4).all(|word| {
                            f32::from_ne_bytes(word.try_into().unwrap()).is_finite()
                        }),
                        "custom vertex stream {name} contains nonfinite floats"
                    );
                }
                Ok(())
            }
            Self::SharedBuffer(buffer) | Self::CopiedBuffer(buffer) => validate_buffer(
                name,
                expected,
                buffer.size(),
                buffer.usage(),
                matches!(self, Self::CopiedBuffer(_)),
            ),
        }
    }
}

pub(super) fn validate_buffer(
    name: &str,
    expected: u64,
    size: u64,
    usage: wgpu::BufferUsages,
    copied: bool,
) -> Result<()> {
    ensure!(
        size == expected,
        "custom vertex stream {name} requires exactly {expected} bytes"
    );
    let required = if copied {
        wgpu::BufferUsages::COPY_SRC
    } else {
        wgpu::BufferUsages::STORAGE
    };
    ensure!(
        usage.contains(required),
        "custom vertex stream {name} requires {required:?} usage"
    );
    Ok(())
}
