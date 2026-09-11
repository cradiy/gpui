use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use gpui::{Bounds, point, px, size};
use gpui_wgpu::Scene3dReadback;

use super::{
    Camera, RenderObject, RenderedFrame, Scene3dChannels, Scene3dPixels, Scene3dReadbackConfig,
    Scene3dReadbackMemory, Scene3dReadbackRegion, lookup,
};

/// One completed pixel-center query from a retained rendered frame.
#[derive(Clone, Debug)]
pub struct FramePick {
    /// Top-left-origin physical coordinates in the source output, not window coordinates.
    pub pixel: [u32; 2],
    /// Full source output dimensions, not the one-pixel readback size.
    pub size: [u32; 2],
    pub hit: Option<FramePickHit>,
    camera: Camera,
}

impl FramePick {
    pub fn camera(&self) -> Camera {
        self.camera
    }
}

/// Nearest surviving rendered surface, independent of CPU mesh queries.
#[derive(Clone, Debug)]
pub struct FramePickHit {
    pub object: RenderObject,
    /// Camera-forward distance, not ray length or hardware depth.
    pub linear_depth: f32,
    pub world_position: [f32; 3],
}

/// Nonblocking single-pixel ID/depth readback with its originating frame metadata.
pub struct FramePickReadback {
    pending: Scene3dReadback,
    objects: Arc<[RenderObject]>,
    camera: Camera,
    size: [u32; 2],
}

impl RenderedFrame {
    /// Reads one physical pixel's ID and linear depth without reading a full image.
    /// Both channels must have been rendered. Out-of-bounds pixels return errors.
    /// Uses 512 staging bytes and eight decoded channel bytes, excluding metadata
    /// and allocation overhead. Shares the renderer's single pending-readback permit.
    pub fn pick(&self, pixel: [u32; 2]) -> Result<FramePickReadback> {
        let config = Scene3dReadbackConfig {
            channels: Scene3dChannels::OBJECT_ID | Scene3dChannels::LINEAR_DEPTH,
            max_staging_bytes: Some(512),
            max_cpu_bytes: Some(8),
        };
        Ok(FramePickReadback {
            pending: self.output.readback_region(
                Scene3dReadbackRegion {
                    origin: pixel,
                    size: [1, 1],
                },
                config,
            )?,
            objects: self.objects.clone(),
            camera: self.camera,
            size: self.output.config().size,
        })
    }
}

impl FramePickReadback {
    pub fn memory(&self) -> Scene3dReadbackMemory {
        self.pending.memory()
    }

    /// Pumps callbacks without waiting. `None` means pending; a completed
    /// background query has `Some(FramePick { hit: None, .. })`. Completion or
    /// failure is terminal. Dropping a request cancels mapping, not submitted work.
    pub fn try_read(&mut self) -> Result<Option<FramePick>> {
        self.pending
            .try_read()?
            .map(|pixels| {
                resolve(
                    self.camera,
                    self.size,
                    self.pending.region().origin,
                    &self.objects,
                    &pixels,
                )
            })
            .transpose()
    }
}

fn resolve(
    camera: Camera,
    output_size: [u32; 2],
    pixel: [u32; 2],
    objects: &[RenderObject],
    pixels: &Scene3dPixels,
) -> Result<FramePick> {
    Scene3dReadbackRegion {
        origin: pixel,
        size: [1, 1],
    }
    .validate(output_size)?;
    ensure!(
        pixels.size == [1, 1],
        "3D picking requires a single-pixel readback"
    );
    let [output_id] = pixels
        .object_ids
        .as_deref()
        .context("3D picking requires object IDs")?
    else {
        anyhow::bail!("3D picking requires one object ID");
    };
    let [depth] = pixels
        .linear_depth
        .as_deref()
        .context("3D picking requires linear depth")?
    else {
        anyhow::bail!("3D picking requires one depth sample");
    };
    let hit = if *output_id == 0 {
        ensure!(
            pixels.depth_background.is_background(*depth),
            "3D pick has background ID with non-background depth"
        );
        None
    } else {
        let object = lookup(objects, *output_id).context("3D pick has an unknown object ID")?;
        let world_position = camera.screen_to_world(
            Bounds::new(
                point(px(0.), px(0.)),
                size(px(output_size[0] as f32), px(output_size[1] as f32)),
            ),
            point(px(pixel[0] as f32 + 0.5), px(pixel[1] as f32 + 0.5)),
            *depth,
        )?;
        Some(FramePickHit {
            object: object.clone(),
            linear_depth: *depth,
            world_position,
        })
    };
    Ok(FramePick {
        pixel,
        size: output_size,
        hit,
        camera,
    })
}

#[cfg(test)]
mod tests;
