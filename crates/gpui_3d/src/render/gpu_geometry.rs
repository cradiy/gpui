use crate::{Aabb, Scene, Scene3dGpuDraw};
use anyhow::{Result, ensure};
use std::{collections::HashSet, sync::Arc};

pub(super) fn with_geometry(scene: &Scene, draws: &[Scene3dGpuDraw]) -> Result<Scene> {
    let mut scene = with_bounds(
        scene,
        draws.iter().map(|draw| (draw.output_id, draw.bounds)),
    )?;
    for draw in draws {
        let object = &mut scene.objects[draw.output_id as usize - 1];
        ensure!(
            !draw.geometry.context().device_lost(),
            "GPU geometry device is lost"
        );
        ensure!(
            Arc::ptr_eq(&object.mesh.0, draw.geometry.base_mesh()),
            "GPU geometry source mesh mismatch for object {}",
            draw.output_id
        );
        object.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(draw.geometry.clone()));
    }
    Ok(scene)
}

pub(super) fn with_bounds(
    scene: &Scene,
    bounds: impl IntoIterator<Item = (u32, [[f32; 3]; 2])>,
) -> Result<Scene> {
    let mut output = scene.clone();
    let mut seen = HashSet::new();
    for (id, bounds) in bounds {
        ensure!(seen.insert(id), "duplicate GPU draw object {id}");
        let index =
            id.checked_sub(1)
                .ok_or_else(|| anyhow::anyhow!("GPU draw IDs start at one"))? as usize;
        let object = output
            .objects
            .get_mut(index)
            .ok_or_else(|| anyhow::anyhow!("GPU draw object {id} is absent from the scene"))?;
        object.render_bounds = Some(
            Aabb::new(bounds[0], bounds[1])
                .ok_or_else(|| anyhow::anyhow!("GPU draw object {id} has invalid bounds"))?,
        );
    }
    if !seen.is_empty() {
        output.preparation_revision = Arc::new(());
    }
    Ok(output)
}
