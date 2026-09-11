use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gpui::{Scene, Scene3dFrame, Scene3dViewportCapabilities, SubtreeLayer};

use super::{RenderRegion, Scene3dRenderer, WgpuAtlas, viewport::visit_scenes};
use crate::{Scene3dCapabilities, Scene3dDeviceCapabilities, WgpuContext, WgpuScene3dPickFrame};

pub(in crate::wgpu_renderer) fn fail_pick_captures(scene: &Scene, error: gpui::SharedString) {
    scene.visit(&mut |scene| {
        for layer in &scene.subtree_layers {
            if let Some(capture) = layer
                .scene3d
                .as_ref()
                .and_then(|frame| frame.pick_capture.as_ref())
            {
                capture.publish::<WgpuScene3dPickFrame>(Err(error.clone()));
            }
        }
    });
}

struct Entry {
    frame: Arc<Scene3dFrame>,
    region: RenderRegion,
    start: [usize; 2],
    output: Arc<WgpuScene3dPickFrame>,
    encoded: AtomicBool,
}

pub(super) struct PickRenderer {
    context: WgpuContext,
    capabilities: Result<Scene3dCapabilities, String>,
    renderers: Option<[Scene3dRenderer; 2]>,
    entries: HashMap<usize, Entry>,
    busy: Arc<AtomicBool>,
}

impl PickRenderer {
    pub(super) fn new(context: WgpuContext) -> Self {
        Self {
            capabilities: Scene3dDeviceCapabilities::query(&context)
                .rendering()
                .map_err(|error| error.to_string()),
            context,
            renderers: None,
            entries: HashMap::new(),
            busy: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn prepare(
        &mut self,
        scene: &Scene,
        size: [u32; 2],
        viewport: Scene3dViewportCapabilities,
    ) -> anyhow::Result<()> {
        self.entries.clear();
        visit_scenes(scene, |scene| {
            for layer in &scene.subtree_layers {
                let Some(frame) = &layer.scene3d else {
                    continue;
                };
                let Some(capture) = &frame.pick_capture else {
                    continue;
                };
                let allocate = (|| -> anyhow::Result<_> {
                    anyhow::ensure!(!self.context.device_lost(), "3D picking device is lost");
                    let capabilities = self
                        .capabilities
                        .as_ref()
                        .map_err(|error| anyhow::anyhow!(error.clone()))?;
                    let bounds = layer.composite.bounds;
                    let region = RenderRegion::viewport(
                        [
                            bounds.origin.x.0,
                            bounds.origin.y.0,
                            bounds.size.width.0,
                            bounds.size.height.0,
                        ],
                        size,
                        frame.viewport_quality.resolution_scale(),
                        viewport.max_texture_dimension,
                    )
                    .ok_or_else(|| {
                        anyhow::anyhow!("3D picking viewport is outside the render surface")
                    })?;
                    let output = WgpuScene3dPickFrame::allocate(
                        self.context.clone(),
                        *capabilities,
                        frame,
                        region.size,
                        region.rect,
                        region.source_rect,
                        self.busy.clone(),
                    )?;
                    Ok(Entry {
                        frame: frame.clone(),
                        region,
                        start: [0; 2],
                        output: Arc::new(output),
                        encoded: AtomicBool::new(false),
                    })
                })();
                match allocate {
                    Ok(entry) => {
                        self.entries.insert(layer as *const _ as usize, entry);
                    }
                    Err(error) => {
                        capture.publish::<WgpuScene3dPickFrame>(Err(format!("{error:#}").into()))
                    }
                }
            }
        });
        if self.entries.is_empty() {
            self.renderers = None;
            return Ok(());
        }
        let device = &self.context.device;
        let queue = &self.context.queue;
        let renderers = self.renderers.get_or_insert_with(|| {
            [wgpu::TextureFormat::R32Uint, wgpu::TextureFormat::R32Float]
                .map(|format| Scene3dRenderer::new(device, queue, format, 1))
        });
        for index in 0..2 {
            if index == 1 {
                let (first, second) = renderers.split_at_mut(1);
                second[0].reuse_resources_from(&first[0]);
            }
            let renderer = &mut renderers[index];
            renderer.prepare_frames(
                device,
                queue,
                self.entries.values().map(|entry| entry.frame.as_ref()),
                self.entries.values().map(|entry| entry.region.size),
            )?;
            let mut start = 0;
            for entry in self.entries.values_mut() {
                entry.start[index] = start;
                start += renderer.plan(&entry.frame).batches.len();
                let mut statistics = entry.output.gpu().draw_statistics();
                statistics += renderer.draw_statistics(&entry.frame);
                Arc::get_mut(&mut entry.output)
                    .unwrap()
                    .set_statistics(statistics);
            }
        }
        Ok(())
    }

    pub(super) fn encode(
        &self,
        layer: &SubtreeLayer,
        atlas: &WgpuAtlas,
        source: &wgpu::TextureView,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        let Some(entry) = self.entries.get(&(layer as *const _ as usize)) else {
            return;
        };
        let output = entry.output.gpu();
        for (index, texture) in [output.object_ids().unwrap(), output.linear_depth().unwrap()]
            .into_iter()
            .enumerate()
        {
            self.renderers.as_ref().unwrap()[index].encode_frame(
                &self.context.device,
                &self.context.queue,
                atlas,
                &entry.frame,
                entry.region,
                entry.start[index],
                Some(source),
                Some(&texture.create_view(&Default::default())),
                encoder,
            );
        }
        entry.encoded.store(true, Ordering::Release);
    }

    pub(super) fn commit(&self, submitted: bool) {
        for renderer in self.renderers.iter().flatten() {
            renderer.commit_uploads(submitted);
        }
        for entry in self.entries.values() {
            if entry.encoded.swap(false, Ordering::AcqRel) && submitted {
                entry
                    .frame
                    .pick_capture
                    .as_ref()
                    .unwrap()
                    .publish(Ok(entry.output.clone()));
            }
        }
    }

    pub(super) fn retain_external_uploads(&self) {
        for renderer in self.renderers.iter().flatten() {
            renderer.retain_external_uploads();
        }
    }
}
