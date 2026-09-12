use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result, ensure};
use gpui_3d::{
    GpuDeformationOutput, Mesh, NodeHandle, Scene3dGpuGeometryMemory, WgpuScene3dGeometry,
};

pub(super) type Sources = HashMap<NodeHandle, (Mesh, WgpuScene3dGeometry)>;

fn admit(inputs: &[(NodeHandle, &Mesh, [u32; 5])], max_bytes: u64) -> Result<()> {
    let mut nodes = HashSet::with_capacity(inputs.len());
    let mut bytes = 0_u64;
    for (node, mesh, sets) in inputs {
        ensure!(nodes.insert(*node), "Duplicate GPU packing node");
        for &set in sets {
            ensure!(
                mesh.uv_at(set, 0).is_some(),
                "GPU packing mesh is missing UV set {set}"
            );
        }
        let memory = Scene3dGpuGeometryMemory::plan(mesh.vertex_count(), mesh.index_count())?;
        bytes = bytes
            .checked_add(memory.total_bytes)
            .context("GPU packing payload overflow")?;
    }
    ensure!(
        bytes <= max_bytes,
        "GPU packing batch requires {bytes} bytes, exceeding the configured budget"
    );
    Ok(())
}

/// Builds one current source per node without modifying the previous cache.
/// The budget counts each complete source plus one result, including reused inputs.
pub(super) fn prepare(
    previous: &Sources,
    inputs: &[(NodeHandle, &GpuDeformationOutput, [u32; 5])],
    max_bytes: u64,
) -> Result<Sources> {
    let meshes: Vec<_> = inputs
        .iter()
        .map(|(node, output, sets)| (*node, output.base_mesh(), *sets))
        .collect();
    admit(&meshes, max_bytes)?;
    if let Some((_, first, _)) = inputs.first() {
        ensure!(!first.context().device_lost(), "GPU packing device is lost");
        for (node, output, _) in inputs {
            ensure!(
                std::sync::Arc::ptr_eq(&first.context().device, &output.context().device),
                "GPU packing outputs belong to different devices"
            );
            if let Some((_, source)) = previous.get(node) {
                ensure!(
                    std::sync::Arc::ptr_eq(&source.context().device, &output.context().device),
                    "GPU packing source belongs to a different device"
                );
            }
        }
    }
    inputs
        .iter()
        .map(|&(node, output, sets)| {
            let mesh = output.base_mesh();
            let source = match previous.get(&node) {
                Some((old_mesh, source)) if old_mesh.ptr_eq(mesh) && source.uv_sets() == sets => {
                    source.clone()
                }
                Some((_, source)) => output.rebind_render_source(source, sets, Some(max_bytes))?,
                None => output.render_source(sets, Some(max_bytes))?,
            };
            Ok((node, (mesh.clone(), source)))
        })
        .collect()
}

#[cfg(test)]
#[path = "packing/tests.rs"]
mod tests;
