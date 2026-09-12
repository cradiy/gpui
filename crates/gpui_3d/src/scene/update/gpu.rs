use super::*;
use crate::{MeshPassExpansion, Scene3dMaterialSnapshot, WgpuContext};
use gpui_wgpu::wgpu;

#[cfg(test)]
mod tests;

impl ObjectUpdate {
    /// Replaces render-ready GPU vertices and conservative local bounds together.
    /// CPU queries use the result's base mesh; use rendered ID/depth queries for its GPU pose.
    /// Standard UV/color updates are carried by the packed geometry result.
    pub fn gpu_geometry(
        mut self,
        geometry: Arc<crate::Scene3dGpuGeometry>,
        bounds: crate::Aabb,
    ) -> Self {
        self.geometry = Some(Geometry::Gpu(geometry, bounds));
        self
    }
}

fn check_device<'a>(
    context: &mut Option<&'a WgpuContext>,
    candidate: &'a WgpuContext,
) -> Result<()> {
    ensure!(!candidate.device_lost(), "scene resource device is lost");
    if let Some(current) = *context {
        ensure!(
            Arc::ptr_eq(&current.device, &candidate.device),
            "scene resources belong to different devices"
        );
    }
    *context = Some(candidate);
    Ok(())
}

pub(super) fn validate(scene: &Scene) -> Result<()> {
    let mut context = None;
    for input in scene.geometry_inputs()? {
        let object = &scene.objects[input.object_index];
        if let Some(resource) = &object.gpu_geometry {
            let geometry = resource
                .downcast_ref::<crate::Scene3dGpuGeometry>()
                .ok_or_else(|| anyhow::anyhow!("unsupported GPU geometry resource"))?;
            check_device(&mut context, geometry.context())?;
            ensure!(
                Arc::ptr_eq(&object.mesh.0, geometry.base_mesh()),
                "GPU geometry source mesh mismatch"
            );
            ensure!(
                input.uv_sets == geometry.uv_sets(),
                "GPU geometry material coordinate mismatch"
            );
            ensure!(
                object.render_bounds.is_some(),
                "GPU geometry requires conservative bounds"
            );
        }
        for (resource, expansion) in object
            .material
            .custom_material
            .iter()
            .map(|resource| (resource, None))
            .chain(
                object
                    .material
                    .mesh_passes
                    .iter()
                    .map(|pass| (&pass.material, pass.expansion.as_ref())),
            )
        {
            let snapshot = resource
                .downcast_ref::<Scene3dMaterialSnapshot>()
                .ok_or_else(|| anyhow::anyhow!("unsupported material resource"))?;
            check_device(&mut context, snapshot.source().context())?;
            snapshot.validate_vertex_count(object.mesh.vertex_count())?;
            if let Some(MeshPassExpansion {
                weight_attribute: Some(name),
                ..
            }) = expansion
            {
                ensure!(
                    snapshot
                        .source()
                        .program()
                        .vertex_attributes()
                        .iter()
                        .any(|attribute| attribute.name == name.as_ref()
                            && attribute.format == wgpu::VertexFormat::Float32),
                    "mesh pass width requires a declared Float32 attribute"
                );
            }
        }
    }
    Ok(())
}
