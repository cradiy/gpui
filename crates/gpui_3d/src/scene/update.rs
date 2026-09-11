use super::*;
use anyhow::{Result, ensure};
use std::collections::HashSet;

#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
mod gpu;
#[cfg(test)]
mod tests;

/// Final values for one scene object. Omitted properties retain their current values.
/// Geometry replacements include their render bounds; materials retain their custom streams.
#[derive(Clone, Default)]
pub struct ObjectUpdate {
    world: Option<AffineTransform>,
    geometry: Option<Geometry>,
    material: Option<Material>,
}

#[derive(Clone)]
enum Geometry {
    Cpu(Mesh),
    #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
    Gpu(Arc<crate::Scene3dGpuGeometry>, crate::Aabb),
}

impl ObjectUpdate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the final object-to-world matrix without re-evaluating a hierarchy.
    pub fn world(mut self, world: AffineTransform) -> Self {
        self.world = Some(world);
        self
    }

    /// Replaces CPU geometry, including UV/color attributes, and clears GPU geometry/bounds.
    pub fn mesh(mut self, mesh: Mesh) -> Self {
        self.geometry = Some(Geometry::Cpu(mesh));
        self
    }

    /// Replaces the complete material, including custom streams and additional passes.
    pub fn material(mut self, material: Material) -> Self {
        self.material = Some(material);
        self
    }
}

impl Scene {
    /// Publishes final object values together, leaving this scene unchanged on success or error.
    /// Targets are output IDs from this scene's `geometry_inputs()` (object index plus one).
    /// IDs are scene-local, not stable handles across graph evaluations. Duplicate or absent
    /// targets fail. Object order, node/application identities, camera and lighting are retained.
    /// Validates scene parameters and resource metadata without resolving images or submitting
    /// GPU commands. Target-specific limits, image readiness and pipeline creation remain render
    /// checks. External shared buffers must remain immutable while retained scenes use them.
    pub fn with_object_updates(
        &self,
        updates: impl IntoIterator<Item = (u32, ObjectUpdate)>,
    ) -> Result<Self> {
        let mut output = self.clone();
        let mut seen = HashSet::new();
        for (id, update) in updates {
            ensure!(seen.insert(id), "duplicate object update {id}");
            let index = id
                .checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("object IDs start at one"))?;
            let object = output
                .objects
                .get_mut(index as usize)
                .ok_or_else(|| anyhow::anyhow!("object update {id} is absent from the scene"))?;
            if let Some(world) = update.world {
                object.world = Some(world);
            }
            if let Some(material) = update.material {
                object.material = material;
            }
            match update.geometry {
                None => {}
                Some(Geometry::Cpu(mesh)) => {
                    object.mesh = mesh;
                    object.gpu_geometry = None;
                    object.render_bounds = None;
                }
                #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
                Some(Geometry::Gpu(geometry, bounds)) => {
                    object.mesh = Mesh(geometry.base_mesh().clone(), Arc::default());
                    object.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(geometry));
                    object.render_bounds = Some(bounds);
                }
            }
        }
        output.plan_frame(1., None)?;
        #[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
        gpu::validate(&output)?;
        if !seen.is_empty() {
            output.preparation_revision = Arc::new(());
            output.spatial_index = Arc::default();
            output.spatial_source = None;
        }
        Ok(output)
    }
}
