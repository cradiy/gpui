use super::{GpuDeformationBounds, GpuDeformationBoundsReadback, GpuDeformationOutput};
use crate::Aabb;
use anyhow::{Context as _, Result, ensure};
use gpui_wgpu::{Scene3dGeometryStatusReadback, Scene3dGpuGeometry, WgpuScene3dGeometry};
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// A validated packed geometry and its mesh-local bounds from the same deformation output.
/// Material clipping and scene/world transforms are applied separately during submission.
#[derive(Clone)]
pub struct PreparedGpuGeometry {
    geometry: Arc<Scene3dGpuGeometry>,
    bounds: Aabb,
}

impl PreparedGpuGeometry {
    pub fn geometry(&self) -> &Arc<Scene3dGpuGeometry> {
        &self.geometry
    }
    pub fn bounds(&self) -> Aabb {
        self.bounds
    }
}

/// Owns pending packing validation and bounds reduction. No partial result is published.
/// Dropping cancels mapping, not submitted work. Concurrent-request limits are caller-owned.
#[must_use = "retain the preparation until completion or drop it to cancel mapping"]
pub struct GpuGeometryPreparation {
    pending: Option<Pending>,
    working_bytes: u64,
}

struct Pending {
    geometry: Arc<Scene3dGpuGeometry>,
    status: Option<Scene3dGeometryStatusReadback>,
    bounds: Option<GpuDeformationBoundsReadback>,
    value: Option<Aabb>,
}

impl GpuDeformationOutput {
    /// Packs geometry and starts validation/bounds readbacks for this exact output.
    /// The working limit covers new packed vertices, indirect/validation storage and
    /// 96 bytes for bounds reduction and both staging buffers. Existing sources, indices,
    /// deformation inputs, shared pipelines and driver overhead are excluded.
    pub fn prepare_render_geometry(
        &self,
        source: &WgpuScene3dGeometry,
        bounds: &GpuDeformationBounds,
        max_working_bytes: Option<u64>,
    ) -> Result<GpuGeometryPreparation> {
        ensure!(
            !self.context.device_lost(),
            "GPU deformation device is lost"
        );
        ensure!(
            Arc::ptr_eq(&self.context.device, &bounds.context.device),
            "GPU bounds reducer belongs to a different device"
        );
        let memory = source.memory();
        let working_bytes = memory
            .vertex_bytes
            .checked_add(memory.draw_bytes)
            .and_then(|bytes| bytes.checked_add(96))
            .context("GPU preparation payload overflow")?;
        ensure!(
            max_working_bytes.is_none_or(|limit| working_bytes <= limit),
            "GPU geometry preparation requires {working_bytes} working bytes"
        );
        let geometry = Arc::new(self.render_geometry(source)?);
        let status = geometry.request_status(Some(32))?;
        let bounds = bounds.request(self, Some(64))?;
        Ok(GpuGeometryPreparation {
            pending: Some(Pending {
                geometry,
                status: Some(status),
                bounds: Some(bounds),
                value: None,
            }),
            working_bytes,
        })
    }
}

impl GpuGeometryPreparation {
    /// Per-request admitted payload, including packed output and temporary GPU buffers.
    pub fn working_bytes(&self) -> u64 {
        self.working_bytes
    }

    /// Polls without waiting. Returns a result only after both checks succeed.
    /// Failed or completed requests are terminal; earlier prepared outputs are unchanged.
    pub fn try_read(&mut self) -> Result<Option<PreparedGpuGeometry>> {
        let pending = self
            .pending
            .as_mut()
            .context("GPU geometry preparation is finished")?;
        match pending.try_read() {
            Ok(Some(bounds)) => {
                let pending = self.pending.take().unwrap();
                Ok(Some(PreparedGpuGeometry {
                    geometry: pending.geometry,
                    bounds,
                }))
            }
            Ok(None) => Ok(None),
            Err(error) => {
                self.pending.take();
                Err(error)
            }
        }
    }
}

impl Pending {
    fn try_read(&mut self) -> Result<Option<Aabb>> {
        if let Some(request) = &mut self.status
            && let Some(status) = request.try_read()?
        {
            ensure!(status.is_drawable(), "GPU geometry rejected: {status:?}");
            self.status = None;
        }
        if let Some(request) = &mut self.bounds
            && let Some(bounds) = request.try_read()?
        {
            self.value = Some(bounds);
            self.bounds = None;
        }
        Ok(if self.status.is_none() {
            self.value
        } else {
            None
        })
    }
}
