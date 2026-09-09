use super::{Scene3dCapabilities, WgpuContext};
use anyhow::{Result, ensure};
use wgpu::{TextureFormat as F, TextureFormatFeatureFlags as Flags, TextureUsages as Usage};

/// Adapter-advertised and device-enabled features for a 3D texture format.
#[derive(Clone, Copy, Debug)]
pub struct Scene3dFormatCapabilities {
    pub format: F,
    pub adapter: wgpu::TextureFormatFeatures,
    pub device: wgpu::TextureFormatFeatures,
}

/// A snapshot of an existing WGPU context. Querying does not create pipelines,
/// allocate render targets, submit work, or wait for the GPU.
#[derive(Clone, Debug)]
pub struct Scene3dDeviceCapabilities {
    pub adapter_info: wgpu::AdapterInfo,
    pub adapter_features: wgpu::Features,
    pub enabled_features: wgpu::Features,
    pub limits: wgpu::Limits,
    pub downlevel: wgpu::DownlevelCapabilities,
    pub color_atlas_format: F,
    /// Output, depth, image, and environment formats used by the mesh renderer.
    pub formats: Vec<Scene3dFormatCapabilities>,
    /// Effective image-sampler limit; one means isotropic filtering only.
    pub max_image_anisotropy: u16,
}

impl Scene3dDeviceCapabilities {
    pub fn query(context: &WgpuContext) -> Self {
        let enabled_features = context.device.features();
        let downlevel = context.adapter.get_downlevel_capabilities();
        let formats = [
            F::Rgba8Unorm,
            F::Rgba16Float,
            F::R32Uint,
            F::R32Float,
            F::Rgba32Float,
            F::Depth32Float,
            F::Rg16Float,
            F::Bgra8Unorm,
        ]
        .into_iter()
        .map(|format| {
            let adapter = context.adapter.get_texture_format_features(format);
            Scene3dFormatCapabilities {
                format,
                adapter,
                device: enabled_format(format, adapter, enabled_features, downlevel.flags),
            }
        })
        .collect();
        Self {
            adapter_info: context.adapter.get_info(),
            adapter_features: context.adapter.features(),
            enabled_features,
            limits: context.device.limits(),
            max_image_anisotropy: if downlevel
                .flags
                .contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING)
            {
                16
            } else {
                1
            },
            downlevel,
            color_atlas_format: context.color_texture_format(),
            formats,
        }
    }

    pub fn format(&self, format: F) -> Option<&Scene3dFormatCapabilities> {
        self.formats.iter().find(|entry| entry.format == format)
    }

    /// Checks mesh pipeline requirements and returns usable output limits.
    /// Errors name the unsupported format, feature, or device limit. This does
    /// not certify device health, allocation success, or platform presentation.
    pub fn rendering(&self) -> Result<Scene3dCapabilities> {
        rendering(
            &self.limits,
            self.downlevel.flags,
            self.color_atlas_format,
            &self.formats,
        )
    }
}

fn enabled_format(
    format: F,
    adapter: wgpu::TextureFormatFeatures,
    features: wgpu::Features,
    downlevel: wgpu::DownlevelFlags,
) -> wgpu::TextureFormatFeatures {
    if !features.contains(format.required_features()) {
        return wgpu::TextureFormatFeatures {
            allowed_usages: Usage::empty(),
            flags: Flags::empty(),
        };
    }
    if features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        || !downlevel.contains(wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT)
    {
        let mut enabled = adapter;
        if matches!(format, F::R32Float | F::Rg32Float | F::Rgba32Float)
            && !features.contains(wgpu::Features::FLOAT32_FILTERABLE)
        {
            enabled.flags.remove(Flags::FILTERABLE);
        }
        enabled
    } else {
        format.guaranteed_format_features(features)
    }
}

fn rendering(
    limits: &wgpu::Limits,
    downlevel: wgpu::DownlevelFlags,
    color_atlas_format: F,
    formats: &[Scene3dFormatCapabilities],
) -> Result<Scene3dCapabilities> {
    crate::wgpu_renderer::scene3d::validate_device_limits(limits)?;
    ensure!(
        downlevel.contains(wgpu::DownlevelFlags::COMPARISON_SAMPLERS),
        "3D rendering requires comparison samplers for directional shadows"
    );
    let get = |format| {
        formats
            .iter()
            .find(|entry| entry.format == format)
            .map(|entry| entry.device)
    };
    let require = |format, usage, flags| -> Result<()> {
        let support =
            get(format).ok_or_else(|| anyhow::anyhow!("3D format {format:?} was not queried"))?;
        ensure!(
            support.allowed_usages.contains(usage),
            "3D format {format:?} lacks device-enabled usages {:?}",
            usage - support.allowed_usages
        );
        ensure!(
            support.flags.contains(flags),
            "3D format {format:?} lacks device-enabled features {:?}",
            flags - support.flags
        );
        Ok(())
    };
    let output = Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING | Usage::COPY_SRC;
    require(
        color_atlas_format,
        Usage::TEXTURE_BINDING | Usage::COPY_DST,
        Flags::FILTERABLE,
    )?;
    require(F::Rgba8Unorm, output | Usage::COPY_DST, Flags::FILTERABLE)?;
    require(
        F::Rgba16Float,
        output | Usage::COPY_DST,
        Flags::FILTERABLE | Flags::BLENDABLE,
    )?;
    require(F::R32Uint, output, Flags::empty())?;
    require(
        F::Depth32Float,
        Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING,
        Flags::empty(),
    )?;
    require(
        F::Rg16Float,
        Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING,
        Flags::FILTERABLE,
    )?;
    let msaa4 = [F::Rgba16Float, F::Depth32Float].into_iter().all(|format| {
        get(format).is_some_and(|features| features.flags.contains(Flags::MULTISAMPLE_X4))
    });
    Ok(Scene3dCapabilities {
        max_dimension: limits.max_texture_dimension_2d,
        max_pixels: 16_777_216,
        color_msaa4: msaa4,
        linear_color_msaa4: msaa4
            && get(F::Rgba16Float)
                .unwrap()
                .flags
                .contains(Flags::MULTISAMPLE_RESOLVE),
        max_readback_buffer_bytes: limits.max_buffer_size,
        geometry_outputs: limits.max_color_attachment_bytes_per_sample >= 16
            && [F::R32Float, F::Rgba32Float].into_iter().all(|format| {
                get(format).is_some_and(|features| features.allowed_usages.contains(output))
            }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Scene3dChannels as C, Scene3dOutputConfig};

    fn formats() -> Vec<Scene3dFormatCapabilities> {
        [
            F::Rgba8Unorm,
            F::Rgba16Float,
            F::R32Uint,
            F::R32Float,
            F::Rgba32Float,
            F::Depth32Float,
            F::Rg16Float,
            F::Bgra8Unorm,
        ]
        .into_iter()
        .map(|format| {
            let features = format.guaranteed_format_features(wgpu::Features::empty());
            Scene3dFormatCapabilities {
                format,
                adapter: features,
                device: features,
            }
        })
        .collect()
    }

    fn render_caps(
        limits: &wgpu::Limits,
        formats: &[Scene3dFormatCapabilities],
    ) -> Result<Scene3dCapabilities> {
        rendering(limits, wgpu::DownlevelFlags::all(), F::Bgra8Unorm, formats)
    }

    #[test]
    fn scene3d_formats_distinguish_adapter_extensions_from_enabled_features() {
        let format = F::Rgba16Float;
        let mut advertised = format.guaranteed_format_features(wgpu::Features::empty());
        advertised.flags.insert(Flags::MULTISAMPLE_X8);
        let portable = enabled_format(
            format,
            advertised,
            wgpu::Features::empty(),
            wgpu::DownlevelFlags::all(),
        );
        assert!(!portable.flags.contains(Flags::MULTISAMPLE_X8));
        let enabled = enabled_format(
            format,
            advertised,
            wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            wgpu::DownlevelFlags::all(),
        );
        assert!(enabled.flags.contains(Flags::MULTISAMPLE_X8));
        advertised.allowed_usages.remove(Usage::RENDER_ATTACHMENT);
        let downlevel = enabled_format(
            format,
            advertised,
            wgpu::Features::empty(),
            wgpu::DownlevelFlags::all() - wgpu::DownlevelFlags::WEBGPU_TEXTURE_FORMAT_SUPPORT,
        );
        assert!(!downlevel.allowed_usages.contains(Usage::RENDER_ATTACHMENT));
        let unavailable = enabled_format(
            F::Bc1RgbaUnorm,
            advertised,
            wgpu::Features::empty(),
            wgpu::DownlevelFlags::all(),
        );
        assert!(unavailable.allowed_usages.is_empty());
        assert!(unavailable.flags.is_empty());
    }

    #[test]
    fn scene3d_float32_filtering_requires_the_device_feature() {
        for format in [F::R32Float, F::Rg32Float, F::Rgba32Float] {
            let advertised = format.guaranteed_format_features(wgpu::Features::FLOAT32_FILTERABLE);
            for downlevel in [wgpu::DownlevelFlags::all(), wgpu::DownlevelFlags::empty()] {
                let disabled = enabled_format(
                    format,
                    advertised,
                    wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
                    downlevel,
                );
                assert!(!disabled.flags.contains(Flags::FILTERABLE));
                let enabled = enabled_format(
                    format,
                    advertised,
                    wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                        | wgpu::Features::FLOAT32_FILTERABLE,
                    downlevel,
                );
                assert!(enabled.flags.contains(Flags::FILTERABLE));
            }
        }
    }

    #[test]
    fn scene3d_output_queries_match_validation_for_each_channel_combination() {
        let mut formats = formats();
        let limits = wgpu::Limits::downlevel_defaults();
        for pass in 0..4 {
            let caps = render_caps(&limits, &formats).unwrap();
            for bits in 0..=C::all().bits() + 1 {
                let channels = C::from_bits_retain(bits);
                for color_samples in [0, 1, 2, 4, 8] {
                    assert_eq!(
                        caps.color_sample_counts(channels).contains(&color_samples),
                        caps.validate(Scene3dOutputConfig {
                            size: [1, 1],
                            channels,
                            color_samples
                        })
                        .is_ok()
                    );
                }
            }
            match pass {
                0 => formats
                    .iter_mut()
                    .find(|entry| entry.format == F::Rgba16Float)
                    .unwrap()
                    .device
                    .flags
                    .remove(Flags::MULTISAMPLE_RESOLVE),
                1 => formats
                    .iter_mut()
                    .find(|entry| entry.format == F::Depth32Float)
                    .unwrap()
                    .device
                    .flags
                    .remove(Flags::MULTISAMPLE_X4),
                _ => formats
                    .iter_mut()
                    .find(|entry| entry.format == F::Rgba32Float)
                    .unwrap()
                    .device
                    .allowed_usages
                    .remove(Usage::COPY_SRC),
            }
        }
        let caps = render_caps(&limits, &formats).unwrap();
        assert!(!caps.geometry_outputs && !caps.color_msaa4 && !caps.linear_color_msaa4);
        assert_eq!(caps.channels(), C::COLOR | C::LINEAR_COLOR | C::OBJECT_ID);
    }

    #[test]
    fn scene3d_device_requirements_report_unusable_formats_and_pipeline_limits() {
        let original = formats();
        let limits = wgpu::Limits::downlevel_defaults();
        render_caps(&limits, &original).unwrap();
        for (format, usage, flag) in [
            (F::Depth32Float, Usage::RENDER_ATTACHMENT, Flags::empty()),
            (F::Rgba16Float, Usage::empty(), Flags::BLENDABLE),
            (F::Rg16Float, Usage::empty(), Flags::FILTERABLE),
            (F::Bgra8Unorm, Usage::COPY_DST, Flags::empty()),
        ] {
            let mut entries = original.clone();
            let entry = entries
                .iter_mut()
                .find(|entry| entry.format == format)
                .unwrap();
            entry.device.allowed_usages.remove(usage);
            entry.device.flags.remove(flag);
            let error = render_caps(&limits, &entries).unwrap_err().to_string();
            assert!(error.contains(&format!("{format:?}")), "{error}");
        }
        for (name, constrained) in [
            (
                "max_vertex_attributes",
                wgpu::Limits {
                    max_vertex_attributes: 13,
                    ..limits
                },
            ),
            (
                "max_samplers_per_shader_stage",
                wgpu::Limits {
                    max_samplers_per_shader_stage: 6,
                    ..limits
                },
            ),
            (
                "max_uniform_buffer_binding_size",
                wgpu::Limits {
                    max_uniform_buffer_binding_size: 16,
                    ..limits
                },
            ),
            (
                "max_buffer_size",
                wgpu::Limits {
                    max_buffer_size: 16,
                    ..limits
                },
            ),
            (
                "max_texture_array_layers",
                wgpu::Limits {
                    max_texture_array_layers: 1,
                    ..limits
                },
            ),
            (
                "max_texture_dimension_2d",
                wgpu::Limits {
                    max_texture_dimension_2d: 512,
                    ..limits
                },
            ),
        ] {
            let error = render_caps(&constrained, &original)
                .unwrap_err()
                .to_string();
            assert!(error.contains(name), "{error}");
        }
        let error = rendering(
            &limits,
            wgpu::DownlevelFlags::all() - wgpu::DownlevelFlags::COMPARISON_SAMPLERS,
            F::Bgra8Unorm,
            &original,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("comparison samplers"));
        let caps = render_caps(
            &wgpu::Limits {
                max_color_attachment_bytes_per_sample: 8,
                ..limits
            },
            &original,
        )
        .unwrap();
        assert!(!caps.geometry_outputs);
    }
}
