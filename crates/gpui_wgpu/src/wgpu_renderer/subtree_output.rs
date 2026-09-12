use super::*;

impl WgpuRenderer {
    /// Renders each layer to a separate transparent texture for a native compositor.
    /// Targets must match this external renderer's size and format and belong to
    /// its device. Commands are submitted together; mesh caches and picking are
    /// committed only after successful encoding. Layers retain their input order.
    pub fn draw_subtree_layers(
        &mut self,
        layers: &[gpui::SubtreeLayer],
        targets: &[wgpu::Texture],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            layers.len() == targets.len(),
            "one target is required per subtree"
        );
        let mut scene = Scene::default();
        scene.subtree_layers = layers.to_vec();
        let Some(target) = targets.first() else {
            self.clear_scene3d_caches();
            return Ok(());
        };
        for target in targets {
            anyhow::ensure!(
                target.width() == self.surface_config.width
                    && target.height() == self.surface_config.height
                    && target.format() == self.surface_config.format
                    && target.sample_count() == 1
                    && target
                        .usage()
                        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT),
                "subtree target does not match the external renderer"
            );
        }
        let view = target.create_view(&Default::default());
        loop {
            let mut encoder =
                self.resources()
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("native_subtrees"),
                    });
            let encoded = self.encode_external_scene_with_subtree_targets(
                &scene,
                WgpuExternalRenderTarget {
                    texture: target,
                    view: &view,
                    command_encoder: &mut encoder,
                },
                true,
                targets,
            );
            let complete = matches!(encoded, Ok(SceneEncoding::Complete));
            if complete {
                self.resources().queue.submit([encoder.finish()]);
            }
            self.commit_encoded_scene(complete);
            self.commit_scene3d_outputs(complete);
            match encoded? {
                SceneEncoding::Complete => return Ok(()),
                SceneEncoding::InstanceCapacity | SceneEncoding::CaptureCapacity => continue,
            }
        }
    }
}
