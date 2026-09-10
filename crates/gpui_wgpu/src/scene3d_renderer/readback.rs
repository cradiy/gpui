use super::{OutputKind, Scene3dChannels, Scene3dPixels, readback_stride};
use anyhow::{Context as _, Result, ensure};

/// Per-request channel selection and payload admission. `None` disables a
/// payload limit, not device limits or the renderer's pending-readback limit.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dReadbackConfig {
    pub channels: Scene3dChannels,
    pub max_staging_bytes: Option<u64>,
    pub max_cpu_bytes: Option<u64>,
}

impl Scene3dPixels {
    pub(super) fn read_channel(
        &mut self,
        kind: OutputKind,
        stride: u32,
        data: &[u8],
    ) -> Result<()> {
        let width_bytes = u64::from(self.size[0]) * u64::from(kind.bytes_per_pixel());
        ensure!(
            stride > 0 && u64::from(stride) >= width_bytes,
            "3D readback row is shorter than its pixels"
        );
        ensure!(
            data.len() as u64 == u64::from(stride) * u64::from(self.size[1]),
            "3D readback mapped size does not match its rows"
        );
        let count = usize::try_from(u64::from(self.size[0]) * u64::from(self.size[1]))?;
        let samples = || {
            data.chunks_exact(stride as usize).flat_map(|row| {
                row[..width_bytes as usize].chunks_exact(kind.bytes_per_pixel() as usize)
            })
        };
        match kind {
            OutputKind::Color => {
                self.rgba = Some(collect_buffer(
                    count.checked_mul(4).context("3D color size overflow")?,
                    samples().flatten().copied(),
                )?);
            }
            OutputKind::LinearColor => {
                self.linear_rgba = Some(collect_buffer(
                    count,
                    samples().map(|bytes| {
                        std::array::from_fn(|i| {
                            half::f16::from_bits(u16::from_le_bytes(
                                bytes[i * 2..i * 2 + 2].try_into().unwrap(),
                            ))
                            .to_f32()
                        })
                    }),
                )?);
            }
            OutputKind::ObjectId => {
                self.object_ids = Some(collect_buffer(
                    count,
                    samples().map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap())),
                )?);
            }
            OutputKind::LinearDepth => {
                self.linear_depth = Some(collect_buffer(
                    count,
                    samples().map(|bytes| f32::from_le_bytes(bytes.try_into().unwrap())),
                )?);
            }
            OutputKind::WorldNormal => {
                self.world_normals = Some(collect_buffer(
                    count,
                    samples().map(|bytes| {
                        std::array::from_fn(|i| {
                            f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap())
                        })
                    }),
                )?);
            }
        }
        Ok(())
    }
}

fn collect_buffer<T>(count: usize, values: impl Iterator<Item = T>) -> Result<Vec<T>> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(count)
        .context("could not allocate 3D readback pixels")?;
    output.extend(values);
    Ok(output)
}

/// Payload of one readback, excluding source textures, driver/allocator overhead,
/// identity storage, and results retained from earlier requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene3dReadbackMemory {
    /// Sum of all selected GPU copy buffers, including row padding.
    pub staging_bytes: u64,
    /// Sum of tightly packed CPU channel buffers, including HDR widening to f32.
    pub cpu_bytes: u64,
    /// Largest individual padded copy buffer.
    pub max_buffer_bytes: u64,
}

impl Scene3dReadbackConfig {
    pub fn new(channels: Scene3dChannels) -> Self {
        Self {
            channels,
            max_staging_bytes: None,
            max_cpu_bytes: None,
        }
    }

    /// Computes payload and checks configured budgets without a device. Channel
    /// availability and device buffer limits are checked when starting readback.
    pub fn memory(self, size: [u32; 2]) -> Result<Scene3dReadbackMemory> {
        ensure!(
            !self.channels.is_empty() && Scene3dChannels::all().contains(self.channels),
            "3D readback must select known channels"
        );
        ensure!(
            size.iter().all(|&v| v > 0),
            "3D readback dimensions must be positive"
        );
        let mut memory = Scene3dReadbackMemory {
            staging_bytes: 0,
            cpu_bytes: 0,
            max_buffer_bytes: 0,
        };
        for kind in [
            OutputKind::Color,
            OutputKind::LinearColor,
            OutputKind::ObjectId,
            OutputKind::LinearDepth,
            OutputKind::WorldNormal,
        ] {
            if !self.channels.contains(kind.channel()) {
                continue;
            }
            let stride = readback_stride(size[0], kind.bytes_per_pixel());
            ensure!(
                stride <= u64::from(u32::MAX),
                "3D readback row stride exceeds u32"
            );
            let staging = stride
                .checked_mul(u64::from(size[1]))
                .context("3D readback staging size overflow")?;
            let cpu_bpp = if matches!(kind, OutputKind::LinearColor) {
                16
            } else {
                kind.bytes_per_pixel()
            };
            let cpu = u64::from(size[0])
                .checked_mul(u64::from(size[1]))
                .and_then(|pixels| pixels.checked_mul(u64::from(cpu_bpp)))
                .context("3D readback CPU size overflow")?;
            ensure!(
                cpu <= isize::MAX as u64,
                "3D readback CPU channel exceeds addressable allocation"
            );
            memory.staging_bytes = memory
                .staging_bytes
                .checked_add(staging)
                .context("3D readback staging total overflow")?;
            memory.cpu_bytes = memory
                .cpu_bytes
                .checked_add(cpu)
                .context("3D readback CPU total overflow")?;
            memory.max_buffer_bytes = memory.max_buffer_bytes.max(staging);
        }
        ensure!(
            self.max_staging_bytes
                .is_none_or(|limit| memory.staging_bytes <= limit),
            "3D readback requires {} staging bytes, exceeding the configured budget",
            memory.staging_bytes
        );
        ensure!(
            self.max_cpu_bytes
                .is_none_or(|limit| memory.cpu_bytes <= limit),
            "3D readback requires {} CPU bytes, exceeding the configured budget",
            memory.cpu_bytes
        );
        Ok(memory)
    }

    pub(super) fn validate(
        self,
        size: [u32; 2],
        available: Scene3dChannels,
        max_buffer_bytes: u64,
    ) -> Result<Scene3dReadbackMemory> {
        let memory = self.memory(size)?;
        ensure!(
            available.contains(self.channels),
            "3D frame does not contain readback channels {:?}",
            self.channels - available
        );
        ensure!(
            memory.max_buffer_bytes <= max_buffer_bytes,
            "3D readback exceeds the device buffer limit"
        );
        Ok(memory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readback_admission_counts_selected_padded_buffers_and_widened_hdr_pixels() {
        let all = Scene3dReadbackConfig::new(Scene3dChannels::all());
        let memory = all.memory([17, 3]).unwrap();
        assert_eq!(memory.staging_bytes, 4 * 768 + 1536);
        assert_eq!(memory.cpu_bytes, 17 * 3 * (4 + 16 + 4 + 4 + 16));
        assert_eq!(memory.max_buffer_bytes, 1536);
        let mut selected = Scene3dReadbackConfig {
            channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
            max_staging_bytes: Some(1536),
            max_cpu_bytes: Some(408),
        };
        let admitted = selected
            .validate([17, 3], Scene3dChannels::all(), 768)
            .unwrap();
        assert_eq!(admitted.staging_bytes, 1536);
        assert_eq!(admitted.cpu_bytes, 408);
        assert_eq!(admitted.max_buffer_bytes, 768);
        assert!(
            selected
                .validate([17, 3], Scene3dChannels::OBJECT_ID, 768)
                .is_err()
        );
        assert!(
            selected
                .validate([17, 3], Scene3dChannels::all(), 767)
                .is_err()
        );
        selected.max_staging_bytes = Some(1535);
        assert!(selected.memory([17, 3]).is_err());
        selected.max_staging_bytes = None;
        selected.max_cpu_bytes = Some(407);
        assert!(selected.memory([17, 3]).is_err());
        selected.max_cpu_bytes = None;
        assert_eq!(selected.memory([17, 3]).unwrap(), admitted);
        assert!(all.memory([0, 3]).is_err());
        assert!(all.memory([u32::MAX; 2]).is_err());
        assert!(
            Scene3dReadbackConfig::new(Scene3dChannels::empty())
                .memory([1, 1])
                .is_err()
        );
        assert!(
            Scene3dReadbackConfig::new(Scene3dChannels::from_bits_retain(128))
                .memory([1, 1])
                .is_err()
        );
        let hdr = Scene3dReadbackConfig::new(Scene3dChannels::LINEAR_COLOR)
            .memory([32, 2])
            .unwrap();
        assert_eq!(hdr.staging_bytes, 512);
        assert_eq!(hdr.cpu_bytes, 1024);
    }

    #[test]
    fn readback_decoding_preserves_integer_ids_and_color_rows_without_extra_channels() {
        let mut pixels = Scene3dPixels {
            size: [3, 2],
            rgba: None,
            linear_rgba: None,
            object_ids: None,
            linear_depth: None,
            world_normals: None,
        };
        let ids = [0_u32, 16_777_217, u32::MAX, 7, 3, 99];
        let stride = readback_stride(3, 4) as usize;
        let mut mapped = vec![0xcc; stride * 2];
        for (index, id) in ids.iter().enumerate() {
            let offset = index / 3 * stride + index % 3 * 4;
            mapped[offset..offset + 4].copy_from_slice(&id.to_le_bytes());
        }
        pixels
            .read_channel(OutputKind::ObjectId, stride as u32, &mapped)
            .unwrap();
        assert_eq!(pixels.object_ids.as_deref(), Some(ids.as_slice()));
        assert!(
            pixels.rgba.is_none()
                && pixels.linear_depth.is_none()
                && pixels.world_normals.is_none()
        );
        assert!(
            pixels
                .read_channel(OutputKind::ObjectId, 8, &mapped)
                .is_err()
        );
        assert!(
            pixels
                .read_channel(
                    OutputKind::ObjectId,
                    stride as u32,
                    &mapped[..mapped.len() - 1]
                )
                .is_err()
        );
        assert_eq!(pixels.object_ids.as_deref(), Some(ids.as_slice()));
        pixels
            .read_channel(OutputKind::Color, stride as u32, &mapped)
            .unwrap();
        let expected: Vec<_> = ids.into_iter().flat_map(u32::to_le_bytes).collect();
        assert_eq!(pixels.rgba.as_deref(), Some(expected.as_slice()));
        assert!(pixels.linear_rgba.is_none());
        assert!(mapped[12..stride].iter().all(|&byte| byte == 0xcc));
        assert_eq!(pixels.object_ids.as_deref(), Some(ids.as_slice()));
    }
}
