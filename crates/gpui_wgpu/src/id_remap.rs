use crate::WgpuContext;
use anyhow::{Context as _, Result, ensure};
use wgpu::util::DeviceExt as _;

/// Per-invocation payload budgets, independent of device limits and retained outputs.
#[derive(Clone, Copy, Debug)]
pub struct IdRemapConfig {
    pub max_output_bytes: Option<u64>,
    pub max_label_bytes: Option<u64>,
}

impl Default for IdRemapConfig {
    fn default() -> Self {
        Self {
            max_output_bytes: Some(64 * 1024 * 1024),
            max_label_bytes: Some(16 * 1024 * 1024),
        }
    }
}

impl IdRemapConfig {
    /// Unpadded R32Uint output bytes, checked without a GPU.
    pub fn output_bytes(&self, size: [u32; 2]) -> Result<u64> {
        ensure!(
            size.iter().all(|&value| value > 0),
            "ID remap dimensions must be positive"
        );
        let bytes = u64::from(size[0])
            .checked_mul(u64::from(size[1]))
            .and_then(|pixels| pixels.checked_mul(4))
            .context("ID remap output size overflow")?;
        ensure!(
            self.max_output_bytes.is_none_or(|limit| bytes <= limit),
            "ID remap output requires {bytes} bytes, exceeding the configured budget"
        );
        Ok(bytes)
    }

    /// Uploaded table bytes. An empty table uses one zero u32, without a zero-size binding.
    pub fn label_bytes(&self, count: usize) -> Result<u64> {
        ensure!(
            count <= u32::MAX as usize,
            "ID remap label count exceeds u32 IDs"
        );
        let bytes = u64::try_from(count.max(1))?
            .checked_mul(4)
            .context("ID remap label size overflow")?;
        ensure!(
            self.max_label_bytes.is_none_or(|limit| bytes <= limit),
            "ID remap labels require {bytes} bytes, exceeding the configured budget"
        );
        Ok(bytes)
    }

    fn validate_limits(&self, size: [u32; 2], count: usize, limits: &wgpu::Limits) -> Result<()> {
        self.output_bytes(size)?;
        let bytes = self.label_bytes(count)?;
        ensure!(
            size.iter()
                .all(|&value| value <= limits.max_texture_dimension_2d),
            "ID remap output exceeds device dimensions"
        );
        ensure!(
            bytes <= limits.max_buffer_size && bytes <= limits.max_storage_buffer_binding_size,
            "ID remap label table exceeds device buffer limits"
        );
        Ok(())
    }
}

/// Reusable exact integer remapping from R32Uint IDs to a fresh R32Uint texture.
/// Index zero in the table maps source ID one. Source zero and IDs beyond the
/// table become zero; an empty table produces an all-zero image. Inputs and
/// encoders must belong to this context's device. No CPU pixel readback occurs.
pub struct WgpuIdRemapper {
    context: WgpuContext,
    config: IdRemapConfig,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

fn output_usages() -> wgpu::TextureUsages {
    wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::COPY_SRC
}

impl WgpuIdRemapper {
    pub fn new(context: WgpuContext, config: IdRemapConfig) -> Result<Self> {
        ensure!(!context.device_lost(), "ID remap device is lost");
        let device = &context.device;
        ensure!(
            device.limits().max_storage_buffers_per_shader_stage > 0,
            "ID remap requires a fragment storage-buffer binding"
        );
        let format = wgpu::TextureFormat::R32Uint;
        let features = if device
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            context.adapter.get_texture_format_features(format)
        } else {
            format.guaranteed_format_features(device.features())
        };
        ensure!(
            features.allowed_usages.contains(output_usages()),
            "ID remap device lacks R32Uint render, sampling or copy support"
        );
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpui.id_remap.inputs"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(4),
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui.id_remap.layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui.id_remap.shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("id_remap.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpui.id_remap.pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("ID remap pipeline validation failed: {error}");
        }
        Ok(Self {
            context,
            config,
            layout,
            pipeline,
        })
    }

    pub fn context(&self) -> &WgpuContext {
        &self.context
    }
    pub fn config(&self) -> &IdRemapConfig {
        &self.config
    }

    /// Checks metadata and budgets before allocation. Device ownership is checked
    /// by WGPU when binding inputs, not by this metadata-only check.
    pub fn validate_input(&self, input: &wgpu::Texture, label_count: usize) -> Result<()> {
        ensure!(!self.context.device_lost(), "ID remap device is lost");
        validate_shape(
            input.dimension(),
            input.size(),
            input.sample_count(),
            input.usage(),
            input.format(),
        )?;
        self.config.validate_limits(
            [input.width(), input.height()],
            label_count,
            &self.context.device.limits(),
        )
    }

    /// Submits one pass. Input writes must already be submitted on this queue.
    /// Does not wait for completion; subsequent calls never overwrite this output.
    pub fn render(&self, input: &wgpu::Texture, labels: &[u32]) -> Result<wgpu::Texture> {
        self.validate_input(input, labels.len())?;
        let device = &self.context.device;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui.id_remap"),
        });
        let output = self.encode(&mut encoder, input, labels)?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let commands = encoder.finish();
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("ID remap command validation failed: {error}");
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.context.queue.submit(Some(commands));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("ID remap submission failed: {error}");
        }
        Ok(output)
    }

    /// Encodes a pass without finishing or submitting the caller's encoder.
    /// Earlier source writes and later consumers can share this encoder. Source
    /// and output dimensions match; only mip zero is read. The caller owns queue
    /// ordering and finish/submission validation. If recording fails, discard the
    /// encoder. A foreign-device encoder follows WGPU's validation-error behavior.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        input: &wgpu::Texture,
        labels: &[u32],
    ) -> Result<wgpu::Texture> {
        self.validate_input(input, labels.len())?;
        let device = &self.context.device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let view = input.create_view(&wgpu::TextureViewDescriptor {
            mip_level_count: Some(1),
            array_layer_count: Some(1),
            ..Default::default()
        });
        let table = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpui.id_remap.labels"),
            contents: bytemuck::cast_slice(if labels.is_empty() { &[0] } else { labels }),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui.id_remap.inputs"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: table.as_entire_binding(),
                },
            ],
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("ID remap input validation failed: {error}");
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpui.id_remap.output"),
            size: input.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: output_usages(),
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpui.id_remap.pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &inputs, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("ID remap recording failed: {error}");
        }
        Ok(output)
    }
}

fn validate_shape(
    dimension: wgpu::TextureDimension,
    size: wgpu::Extent3d,
    samples: u32,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
) -> Result<()> {
    ensure!(
        dimension == wgpu::TextureDimension::D2
            && size.depth_or_array_layers == 1
            && size.width > 0
            && size.height > 0,
        "ID remap requires a nonempty single-layer 2D texture"
    );
    ensure!(samples == 1, "ID remap input must be single-sampled");
    ensure!(
        usage.contains(wgpu::TextureUsages::TEXTURE_BINDING),
        "ID remap input requires TEXTURE_BINDING usage"
    );
    ensure!(
        format == wgpu::TextureFormat::R32Uint,
        "ID remap input must use R32Uint"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_remap_shader_validates_integer_texture_and_storage_contract() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("id_remap.wgsl")).unwrap();
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap();
        assert_eq!(module.entry_points.len(), 2);
        let input = module
            .global_variables
            .iter()
            .find(|(_, value)| value.name.as_deref() == Some("source"))
            .unwrap()
            .1;
        assert!(matches!(
            module.types[input.ty].inner,
            wgpu::naga::TypeInner::Image {
                class: wgpu::naga::ImageClass::Sampled {
                    kind: wgpu::naga::ScalarKind::Uint,
                    multi: false
                },
                ..
            }
        ));
    }

    #[test]
    fn id_remap_admission_checks_output_and_table_budgets_and_device_limits() {
        let config = IdRemapConfig {
            max_output_bytes: Some(24),
            max_label_bytes: Some(12),
        };
        assert_eq!(config.output_bytes([3, 2]).unwrap(), 24);
        assert_eq!(config.label_bytes(3).unwrap(), 12);
        assert_eq!(config.label_bytes(0).unwrap(), 4);
        assert!(config.output_bytes([4, 2]).is_err());
        assert!(config.output_bytes([0, 2]).is_err());
        assert!(config.label_bytes(4).is_err());
        let unlimited = IdRemapConfig {
            max_output_bytes: None,
            max_label_bytes: None,
        };
        assert!(unlimited.output_bytes([u32::MAX; 2]).is_err());
        let mut limits = wgpu::Limits::default();
        config.validate_limits([3, 2], 3, &limits).unwrap();
        limits.max_texture_dimension_2d = 2;
        assert!(config.validate_limits([3, 2], 3, &limits).is_err());
        limits.max_texture_dimension_2d = 4;
        limits.max_storage_buffer_binding_size = 8;
        assert!(config.validate_limits([3, 2], 3, &limits).is_err());
        limits.max_storage_buffer_binding_size = 16;
        limits.max_buffer_size = 8;
        assert!(config.validate_limits([3, 2], 3, &limits).is_err());
    }

    #[test]
    fn id_remap_rejects_incompatible_input_layouts_without_a_device() {
        use wgpu::{TextureDimension as D, TextureFormat as F, TextureUsages as U};
        let size = wgpu::Extent3d {
            width: 5,
            height: 3,
            depth_or_array_layers: 1,
        };
        validate_shape(D::D2, size, 1, U::TEXTURE_BINDING, F::R32Uint).unwrap();
        for (dimension, extent, samples, usage, format) in [
            (D::D3, size, 1, U::TEXTURE_BINDING, F::R32Uint),
            (
                D::D2,
                wgpu::Extent3d {
                    depth_or_array_layers: 2,
                    ..size
                },
                1,
                U::TEXTURE_BINDING,
                F::R32Uint,
            ),
            (D::D2, size, 4, U::TEXTURE_BINDING, F::R32Uint),
            (D::D2, size, 1, U::COPY_SRC, F::R32Uint),
            (D::D2, size, 1, U::TEXTURE_BINDING, F::R32Float),
        ] {
            assert!(validate_shape(dimension, extent, samples, usage, format).is_err());
        }
    }
}
