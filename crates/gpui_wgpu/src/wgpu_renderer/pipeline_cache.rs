use super::{WgpuBindGroupLayouts, WgpuPipelines, WgpuRenderer};
use crate::WgpuContext;
use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PipelineKey {
    format: wgpu::TextureFormat,
    alpha: wgpu::CompositeAlphaMode,
    samples: u32,
    dual_source: bool,
}

/// Weak ownership keeps device-local sharing from extending renderer lifetimes.
#[derive(Default)]
pub(crate) struct PipelineCache {
    device: Weak<wgpu::Device>,
    layouts: Weak<WgpuBindGroupLayouts>,
    pipelines: HashMap<PipelineKey, Weak<WgpuPipelines>>,
}

impl WgpuRenderer {
    pub(super) fn shared_pipelines(
        context: &WgpuContext,
        format: wgpu::TextureFormat,
        alpha: wgpu::CompositeAlphaMode,
        samples: u32,
        dual_source: bool,
    ) -> (Arc<WgpuBindGroupLayouts>, Arc<WgpuPipelines>) {
        let mut cache = context.pipeline_cache.lock().unwrap();
        let device = Arc::downgrade(&context.device);
        if !cache.device.ptr_eq(&device) {
            *cache = PipelineCache {
                device,
                ..Default::default()
            };
        }
        let layouts = cache.layouts.upgrade().unwrap_or_else(|| {
            let layouts = Arc::new(Self::create_bind_group_layouts(&context.device));
            cache.layouts = Arc::downgrade(&layouts);
            layouts
        });
        let key = PipelineKey {
            format,
            alpha,
            samples,
            dual_source,
        };
        let pipelines = cache
            .pipelines
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                cache
                    .pipelines
                    .retain(|_, pipeline| pipeline.strong_count() > 0);
                let pipelines = Arc::new(Self::create_pipelines(
                    &context.device,
                    &layouts,
                    format,
                    alpha,
                    samples,
                    dual_source,
                ));
                cache.pipelines.insert(key, Arc::downgrade(&pipelines));
                pipelines
            });
        (layouts, pipelines)
    }
}

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn pipelines_share_only_compatible_device_configuration_and_release() -> anyhow::Result<()> {
        let context = WgpuContext::new_headless()?;
        let create = |context: &WgpuContext, alpha| {
            WgpuRenderer::shared_pipelines(
                context,
                wgpu::TextureFormat::Rgba8Unorm,
                alpha,
                1,
                false,
            )
        };
        let (layouts, opaque) = create(&context, wgpu::CompositeAlphaMode::Opaque);
        let (same_layouts, same) = create(&context.clone(), wgpu::CompositeAlphaMode::Opaque);
        assert!(Arc::ptr_eq(&layouts, &same_layouts));
        assert!(Arc::ptr_eq(&opaque, &same));
        let (_, transparent) = create(&context, wgpu::CompositeAlphaMode::PreMultiplied);
        assert!(!Arc::ptr_eq(&opaque, &transparent));
        let weak = Arc::downgrade(&opaque);
        drop(opaque);
        assert!(weak.upgrade().is_some());
        drop(same);
        assert!(
            weak.upgrade().is_none(),
            "a live device must not retain unused pipelines"
        );
        let (_, recreated) = create(&context, wgpu::CompositeAlphaMode::Opaque);
        assert!(!weak.ptr_eq(&Arc::downgrade(&recreated)));
        let other = WgpuContext::new_headless()?;
        let (other_layouts, other_pipeline) = create(&other, wgpu::CompositeAlphaMode::Opaque);
        assert!(!Arc::ptr_eq(&layouts, &other_layouts));
        assert!(!Arc::ptr_eq(&recreated, &other_pipeline));
        Ok(())
    }
}
