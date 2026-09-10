use super::{EffectInstance, GlobalParams, PodBounds, PodTransformationMatrix};
use crate::WgpuContext;
use anyhow::{Context as _, Result, ensure};
use bytemuck::Zeroable as _;
use gpui::{EffectShader, EffectTextureOptions, EffectUniforms};
use wgpu::util::DeviceExt as _;

/// Immutable input sampling and output allocation settings for a texture effect.
#[derive(Clone, Debug)]
pub struct TextureEffectConfig {
    /// One entry per shader input, in shader binding order.
    pub inputs: Vec<EffectTextureOptions>,
    /// Output storage. Float formats retain values outside the display range.
    pub output_format: wgpu::TextureFormat,
    /// Store the effect's straight-alpha result with RGB multiplied by alpha.
    pub premultiplied_alpha: bool,
    /// Maximum unpadded output payload per invocation, not total GPU memory.
    /// `None` removes this budget; device limits still apply.
    pub max_output_bytes: Option<u64>,
}

impl Default for TextureEffectConfig {
    fn default() -> Self {
        Self {
            inputs: vec![EffectTextureOptions {
                premultiplied_alpha: true,
                nearest: false,
            }],
            output_format: wgpu::TextureFormat::Rgba16Float,
            premultiplied_alpha: true,
            max_output_bytes: Some(64 * 1024 * 1024),
        }
    }
}

impl TextureEffectConfig {
    /// Validates output dimensions, format and byte budget without creating a device.
    /// Returns the unpadded output payload; device support is checked by the processor.
    pub fn output_bytes(&self, size: [u32; 2]) -> Result<u64> {
        use wgpu::TextureFormat as F;
        let stride = match self.output_format {
            F::Rgba8Unorm | F::Bgra8Unorm | F::R32Float => 4,
            F::Rgba16Float | F::Rg32Float => 8,
            F::Rgba32Float => 16,
            _ => anyhow::bail!(
                "unsupported texture effect output format {:?}",
                self.output_format
            ),
        };
        ensure!(
            size.into_iter().all(|v| v > 0),
            "texture effect dimensions must be positive"
        );
        let bytes = u64::from(size[0])
            .checked_mul(u64::from(size[1]))
            .and_then(|pixels| pixels.checked_mul(stride))
            .context("texture effect output size overflow")?;
        ensure!(
            self.max_output_bytes.is_none_or(|limit| bytes <= limit),
            "texture effect output requires {bytes} bytes, exceeding the configured budget"
        );
        Ok(bytes)
    }
}

/// Executes a reusable `EffectShader` directly on one, two, or four GPU textures.
///
/// All inputs and command encoders must belong to this context's device. `render`
/// submits on its queue; `encode` appends work for caller-controlled submission.
/// Neither waits for GPU completion or reads pixels back. Each result owns fresh
/// storage and remains valid after subsequent calls or dropping the processor.
/// No native window, atlas, or UI layout is used.
pub struct WgpuTextureEffect {
    context: WgpuContext,
    config: TextureEffectConfig,
    globals_layout: wgpu::BindGroupLayout,
    inputs_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
}

impl WgpuTextureEffect {
    /// Compiles a single image-effect function and validates the selected device format.
    /// Mask shaders and procedural shaders without images are not accepted.
    pub fn new(
        context: WgpuContext,
        shader: &EffectShader,
        config: TextureEffectConfig,
    ) -> Result<Self> {
        ensure!(!context.device_lost(), "texture effect device is lost");
        ensure!(
            !shader.is_mask() && matches!(shader.image_count(), 1 | 2 | 4),
            "texture effects require a one-, two-, or four-image shader"
        );
        ensure!(
            config.inputs.len() == usize::from(shader.image_count()),
            "texture effect input options must match the shader image count"
        );
        // A zero budget can intentionally reject every render without preventing compilation.
        let mut format_config = config.clone();
        format_config.max_output_bytes = None;
        format_config.output_bytes([1, 1])?;
        let device = &context.device;
        let format_features = if device
            .features()
            .contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES)
        {
            context
                .adapter
                .get_texture_format_features(config.output_format)
        } else {
            config
                .output_format
                .guaranteed_format_features(device.features())
        };
        ensure!(
            format_features.allowed_usages.contains(output_usages()),
            "texture effect output format {:?} lacks required device usages",
            config.output_format
        );
        let source = gpui::compose_texture_effect_wgsl(shader, &config.inputs);
        let module = wgpu::naga::front::wgsl::parse_str(&source)
            .map_err(|error| anyhow::anyhow!("texture effect WGSL parse error: {error}"))?;
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .context("texture effect WGSL validation failed")?;

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpui.texture_effect.globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(
                        std::mem::size_of::<GlobalParams>() as u64
                    ),
                },
                count: None,
            }],
        });
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: wgpu::BufferSize::new(
                    std::mem::size_of::<EffectInstance>() as u64
                ),
            },
            count: None,
        }];
        for binding in [1, 3, 4, 5].into_iter().take(config.inputs.len()) {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
        }
        let inputs_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpui.texture_effect.inputs"),
            entries: &entries,
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpui.texture_effect.layout"),
            bind_group_layouts: &[Some(&globals_layout), Some(&inputs_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpui.texture_effect.shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpui.texture_effect.pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_effect"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_effect"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.output_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("texture effect pipeline validation failed: {error}");
        }
        Ok(Self {
            context,
            config,
            globals_layout,
            inputs_layout,
            pipeline,
        })
    }

    /// Immutable sampling, output-format and allocation settings.
    pub fn config(&self) -> &TextureEffectConfig {
        &self.config
    }

    /// Device and queue used by the processor and its caller-owned command encoders.
    pub fn context(&self) -> &WgpuContext {
        &self.context
    }

    /// Renders complete level-zero inputs into a fresh single-sample 2D texture.
    /// Inputs may have different sizes. Color-space conversion is shader-owned;
    /// R32Float depth can be sampled without filterable-float device features.
    /// Integer IDs, depth/stencil formats, array textures and multisampled inputs
    /// are rejected. Uniform dimensions and time are caller-supplied physical units.
    /// Input writes must already be submitted on this queue; use [`Self::encode`]
    /// when an input is produced by work still held in a command encoder.
    pub fn render(
        &self,
        inputs: &[&wgpu::Texture],
        size: [u32; 2],
        uniforms: EffectUniforms,
        time: f32,
    ) -> Result<wgpu::Texture> {
        ensure!(!self.context.device_lost(), "texture effect device is lost");
        let device = &self.context.device;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpui.texture_effect"),
        });
        let output = self.encode(&mut encoder, inputs, size, uniforms, time)?;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let commands = encoder.finish();
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("texture effect command validation failed: {error}");
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        self.context.queue.submit(Some(commands));
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("texture effect submission failed: {error}");
        }
        Ok(output)
    }

    /// Appends one effect pass without finishing or submitting the caller's encoder.
    /// Uses the same input, dimension and output contracts as [`Self::render`].
    /// Inputs may be written by earlier passes in this encoder or by commands that
    /// the caller submits first on the same queue. The returned texture can feed
    /// later passes in this encoder; its pixels are not ready before submission.
    ///
    /// Dropping an unfinished encoder cancels its work. Metadata and input binding
    /// validation occur before recording the pass. If encoding fails after recording
    /// begins, discard the encoder; its earlier commands cannot be rolled back.
    /// The caller owns finish/submission validation and resource ordering.
    /// Encoder ownership cannot be inspected through WGPU's public API; supplying
    /// a different device's encoder follows WGPU's validation-error behavior.
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        inputs: &[&wgpu::Texture],
        size: [u32; 2],
        uniforms: EffectUniforms,
        time: f32,
    ) -> Result<wgpu::Texture> {
        ensure!(!self.context.device_lost(), "texture effect device is lost");
        self.config.output_bytes(size)?;
        let device = &self.context.device;
        ensure!(
            size.into_iter()
                .all(|v| v <= device.limits().max_texture_dimension_2d),
            "texture effect output exceeds device dimensions"
        );
        ensure!(
            time.is_finite() && uniforms.slots().iter().flatten().all(|v| v.is_finite()),
            "texture effect uniforms and time must be finite"
        );
        ensure!(
            inputs.len() == self.config.inputs.len(),
            "texture effect input count mismatch"
        );
        for (index, texture) in inputs.iter().enumerate() {
            validate_input(
                texture.dimension(),
                texture.size(),
                texture.sample_count(),
                texture.usage(),
                texture.format(),
            )
            .with_context(|| format!("invalid texture effect input {index}"))?;
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let views: Vec<_> = inputs
            .iter()
            .map(|texture| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    mip_level_count: Some(1),
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let mut instance = EffectInstance::zeroed();
        instance.bounds = PodBounds {
            origin: [0.; 2],
            size: size.map(|v| v as f32),
        };
        instance.effect_bounds = instance.bounds;
        instance.content_mask = instance.bounds;
        instance.transformation =
            PodTransformationMatrix::from(gpui::TransformationMatrix::default());
        instance.opacity = 1.;
        instance.time = time;
        instance.uniforms = *uniforms.slots();
        for (bounds, texture) in [
            &mut instance.image_bounds,
            &mut instance.second_image_bounds,
            &mut instance.third_image_bounds,
            &mut instance.fourth_image_bounds,
        ]
        .into_iter()
        .zip(inputs)
        {
            *bounds = PodBounds {
                origin: [0.; 2],
                size: [texture.width() as f32, texture.height() as f32],
            };
        }
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpui.texture_effect.instance"),
            contents: bytemuck::bytes_of(&instance),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let globals = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpui.texture_effect.globals"),
            contents: bytemuck::bytes_of(&GlobalParams {
                viewport_size: size.map(|v| v as f32),
                premultiplied_alpha: u32::from(self.config.premultiplied_alpha),
                pad: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let globals = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui.texture_effect.globals"),
            layout: &self.globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals.as_entire_binding(),
            }],
        });
        let mut entries = vec![wgpu::BindGroupEntry {
            binding: 0,
            resource: instance_buffer.as_entire_binding(),
        }];
        entries.extend(views.iter().zip([1, 3, 4, 5]).map(|(view, binding)| {
            wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            }
        }));
        let inputs = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui.texture_effect.inputs"),
            layout: &self.inputs_layout,
            entries: &entries,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("texture effect input validation failed: {error}");
        }
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let output = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpui.texture_effect.output"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.config.output_format,
            usage: output_usages(),
            view_formats: &[],
        });
        let view = output.create_view(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpui.texture_effect"),
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
            pass.set_bind_group(0, &globals, &[]);
            pass.set_bind_group(1, &inputs, &[]);
            pass.draw(0..4, 0..1);
        }
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("texture effect encoding failed: {error}");
        }
        Ok(output)
    }
}

fn output_usages() -> wgpu::TextureUsages {
    wgpu::TextureUsages::RENDER_ATTACHMENT
        | wgpu::TextureUsages::TEXTURE_BINDING
        | wgpu::TextureUsages::COPY_SRC
}

fn validate_input(
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
            && size.height > 0
            && samples == 1,
        "expected a nonempty single-sample 2D texture with one layer"
    );
    ensure!(
        usage.contains(wgpu::TextureUsages::TEXTURE_BINDING),
        "texture lacks TEXTURE_BINDING usage"
    );
    ensure!(
        matches!(
            format.sample_type(None, None),
            Some(wgpu::TextureSampleType::Float { .. })
        ),
        "expected a float-sampled color or data format, got {format:?}"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_effect_shaders_validate_for_color_and_numeric_inputs() {
        let four = EffectShader::wgsl_four_images(
            "fn effect(i: EffectInput, p: EffectParams) -> vec4<f32> { return sample_effect_image_cover(i, i.uv) + sample_effect_second_image_cover(i, i.uv) + sample_effect_third_image_cover(i, i.uv) + load_effect_fourth_image(vec2<i32>(i.position)); }",
        );
        for shader in [
            gpui_effects::subtree_identity_shader(),
            gpui_effects::subtree_blur_shader(),
            gpui_effects::hdr_tone_map_shader(),
            gpui_effects::depth_fog_shader(),
            gpui_effects::bloom_composite_shader(),
            four,
        ] {
            for premultiplied_alpha in [false, true] {
                for nearest in [false, true] {
                    let mut inputs = vec![
                        EffectTextureOptions {
                            premultiplied_alpha,
                            nearest
                        };
                        usize::from(shader.image_count())
                    ];
                    if inputs.len() > 1 {
                        inputs[1] = EffectTextureOptions {
                            premultiplied_alpha: !premultiplied_alpha,
                            nearest: !nearest,
                        };
                    }
                    let source = gpui::compose_texture_effect_wgsl(&shader, &inputs);
                    let module = wgpu::naga::front::wgsl::parse_str(&source).unwrap();
                    wgpu::naga::valid::Validator::new(
                        wgpu::naga::valid::ValidationFlags::all(),
                        wgpu::naga::valid::Capabilities::all(),
                    )
                    .validate(&module)
                    .unwrap();
                    let span = module
                        .types
                        .iter()
                        .find_map(|(_, ty)| {
                            if ty.name.as_deref() == Some("EffectInstance")
                                && let wgpu::naga::TypeInner::Struct { span, .. } = ty.inner
                            {
                                Some(span)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    assert_eq!(span as usize, std::mem::size_of::<EffectInstance>());
                }
            }
        }
    }

    #[test]
    fn texture_effect_admission_checks_payload_and_numeric_formats() {
        for (format, stride) in [
            (wgpu::TextureFormat::Rgba8Unorm, 4),
            (wgpu::TextureFormat::Rgba16Float, 8),
            (wgpu::TextureFormat::Rgba32Float, 16),
            (wgpu::TextureFormat::R32Float, 4),
        ] {
            let bytes = 13 * 7 * stride;
            let mut config = TextureEffectConfig {
                output_format: format,
                max_output_bytes: Some(bytes),
                ..Default::default()
            };
            assert_eq!(config.output_bytes([13, 7]).unwrap(), bytes);
            config.max_output_bytes = Some(bytes - 1);
            assert!(config.output_bytes([13, 7]).is_err());
            config.max_output_bytes = None;
            assert_eq!(config.output_bytes([13, 7]).unwrap(), bytes);
            assert!(config.output_bytes([0, 7]).is_err());
            assert!(config.output_bytes([u32::MAX; 2]).is_err());
        }
        for format in [
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureFormat::Depth32Float,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            assert!(
                TextureEffectConfig {
                    output_format: format,
                    ..Default::default()
                }
                .output_bytes([1, 1])
                .is_err()
            );
        }
        let size = wgpu::Extent3d {
            width: 13,
            height: 7,
            depth_or_array_layers: 1,
        };
        let usage = wgpu::TextureUsages::TEXTURE_BINDING;
        for format in [
            wgpu::TextureFormat::R32Float,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Rgba32Float,
            wgpu::TextureFormat::Rgba8UnormSrgb,
        ] {
            assert!(validate_input(wgpu::TextureDimension::D2, size, 1, usage, format).is_ok());
            assert!(validate_input(wgpu::TextureDimension::D2, size, 4, usage, format).is_err());
            assert!(
                validate_input(
                    wgpu::TextureDimension::D2,
                    wgpu::Extent3d {
                        depth_or_array_layers: 2,
                        ..size
                    },
                    1,
                    usage,
                    format
                )
                .is_err()
            );
            assert!(validate_input(wgpu::TextureDimension::D3, size, 1, usage, format).is_err());
            assert!(
                validate_input(
                    wgpu::TextureDimension::D2,
                    size,
                    1,
                    wgpu::TextureUsages::COPY_SRC,
                    format
                )
                .is_err()
            );
        }
        for format in [
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureFormat::R32Sint,
            wgpu::TextureFormat::Depth32Float,
        ] {
            assert!(validate_input(wgpu::TextureDimension::D2, size, 1, usage, format).is_err());
        }
    }
}
