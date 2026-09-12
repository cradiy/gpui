use super::{Scene3dChannels, Scene3dOutputConfig};
use anyhow::{Context as _, Result, ensure};

/// Unpadded texture payload for one direct-render request, independent of cache hits.
/// Excludes earlier outputs, transient overlap, driver overhead, readback buffers,
/// geometry, images, environments, and pipeline resources. Not physical GPU usage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scene3dTargetMemory {
    pub output_bytes: u64,
    pub attachment_bytes: u64,
    pub shadow_bytes: u64,
    pub total_bytes: u64,
}

impl Scene3dOutputConfig {
    /// Calculates target payload without a device, allocation, or submission.
    /// Shadow resolution, when supplied, must be a power of two in 256..=4096.
    /// Geometry-only outputs do not allocate a shadow map. Device support and
    /// renderer budgets are checked separately by `validate_target_memory`.
    pub fn target_memory(self, shadow_resolution: Option<u32>) -> Result<Scene3dTargetMemory> {
        ensure!(
            self.size.into_iter().all(|size| size > 0),
            "3D output dimensions must be positive"
        );
        ensure!(
            !self.channels.is_empty() && Scene3dChannels::all().contains(self.channels),
            "3D outputs must select known channels"
        );
        ensure!(
            matches!(self.color_samples, 1 | 4),
            "3D color sampling must be 1 or 4"
        );
        ensure!(
            shadow_resolution
                .is_none_or(|size| size.is_power_of_two() && (256..=4096).contains(&size)),
            "invalid shadow map resolution"
        );
        let pixels = u64::from(self.size[0]) * u64::from(self.size[1]);
        let bytes = |per_pixel| {
            pixels
                .checked_mul(per_pixel)
                .context("3D target byte count overflow")
        };
        let mut output_stride = 0;
        for (channel, stride) in [
            (Scene3dChannels::COLOR, 4),
            (Scene3dChannels::LINEAR_COLOR, 8),
            (Scene3dChannels::OBJECT_ID, 4),
            (Scene3dChannels::LINEAR_DEPTH, 4),
            (Scene3dChannels::WORLD_NORMAL, 16),
        ] {
            if self.channels.contains(channel) {
                output_stride += stride;
            }
        }
        let mut attachment_stride = if self.channels.shaded() {
            12 * u64::from(self.color_samples)
        } else {
            0
        };
        for channel in [
            Scene3dChannels::OBJECT_ID,
            Scene3dChannels::LINEAR_DEPTH,
            Scene3dChannels::WORLD_NORMAL,
        ] {
            if self.channels.contains(channel) {
                attachment_stride += 4;
            }
        }
        let shadow_bytes = shadow_resolution
            .filter(|_| self.channels.shaded())
            .map_or(0, |size| u64::from(size).pow(2) * 4);
        let output_bytes = bytes(output_stride)?;
        let attachment_bytes = bytes(attachment_stride)?;
        let total_bytes = output_bytes
            .checked_add(attachment_bytes)
            .and_then(|sum| sum.checked_add(shadow_bytes))
            .context("3D target byte count overflow")?;
        Ok(Scene3dTargetMemory {
            output_bytes,
            attachment_bytes,
            shadow_bytes,
            total_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene3d_target_memory_counts_shared_color_attachments_and_independent_geometry() {
        let config = Scene3dOutputConfig {
            size: [13, 7],
            channels: Scene3dChannels::COLOR,
            color_samples: 4,
        };
        let color = config.target_memory(None).unwrap();
        assert_eq!(
            (
                color.output_bytes,
                color.attachment_bytes,
                color.total_bytes
            ),
            (364, 4368, 4732)
        );
        let both = Scene3dOutputConfig {
            channels: Scene3dChannels::COLOR | Scene3dChannels::LINEAR_COLOR,
            ..config
        }
        .target_memory(None)
        .unwrap();
        assert_eq!(both.attachment_bytes, color.attachment_bytes);
        assert_eq!(both.output_bytes - color.output_bytes, 728);
        let all = Scene3dOutputConfig {
            channels: Scene3dChannels::all(),
            ..config
        }
        .target_memory(Some(256))
        .unwrap();
        assert_eq!(
            (
                all.output_bytes,
                all.attachment_bytes,
                all.shadow_bytes,
                all.total_bytes
            ),
            (3276, 5460, 262144, 270880)
        );
        let single = Scene3dOutputConfig {
            color_samples: 1,
            channels: Scene3dChannels::all(),
            ..config
        }
        .target_memory(Some(256))
        .unwrap();
        assert_eq!(single.output_bytes, all.output_bytes);
        assert_eq!(all.attachment_bytes - single.attachment_bytes, 3276);
        let geometry = Scene3dOutputConfig {
            channels: Scene3dChannels::OBJECT_ID
                | Scene3dChannels::LINEAR_DEPTH
                | Scene3dChannels::WORLD_NORMAL,
            ..config
        };
        let four = geometry.target_memory(Some(4096)).unwrap();
        assert_eq!(four.shadow_bytes, 0);
        assert_eq!(
            four,
            Scene3dOutputConfig {
                color_samples: 1,
                ..geometry
            }
            .target_memory(None)
            .unwrap()
        );
        assert_eq!((four.output_bytes, four.attachment_bytes), (2184, 1092));
        let doubled = Scene3dOutputConfig {
            size: [26, 14],
            ..geometry
        }
        .target_memory(None)
        .unwrap();
        assert_eq!(doubled.total_bytes, four.total_bytes * 4);
    }

    #[test]
    fn scene3d_target_memory_rejects_invalid_shapes_and_overflow_before_allocation() {
        let valid = Scene3dOutputConfig::new([16, 16]);
        for invalid in [
            Scene3dOutputConfig {
                size: [0, 16],
                ..valid
            },
            Scene3dOutputConfig {
                size: [u32::MAX; 2],
                ..valid
            },
            Scene3dOutputConfig {
                size: [1 << 30, 1 << 31],
                channels: Scene3dChannels::OBJECT_ID,
                color_samples: 1,
            },
            Scene3dOutputConfig {
                channels: Scene3dChannels::empty(),
                ..valid
            },
            Scene3dOutputConfig {
                channels: Scene3dChannels::from_bits_retain(128),
                ..valid
            },
            Scene3dOutputConfig {
                color_samples: 2,
                ..valid
            },
        ] {
            assert!(invalid.target_memory(None).is_err());
        }
        for shadow in [0, 255, 257, 8192] {
            assert!(valid.target_memory(Some(shadow)).is_err());
        }
    }
}
