use super::*;
use gpui_3d::{GpuDeformationLimits, Node, SceneGraph, WgpuContext};

#[test]
fn batch_admission_counts_shared_mesh_occurrences_and_checks_all_coordinates() -> Result<()> {
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new())?;
    let b = graph.insert(None, Node::new())?;
    let mesh = Mesh::plane();
    let bytes =
        Scene3dGpuGeometryMemory::plan(mesh.vertex_count(), mesh.index_count())?.total_bytes;
    let inputs = [(a, &mesh, [0; 5]), (b, &mesh, [0; 5])];
    admit(&inputs, bytes * 2)?;
    assert!(admit(&inputs, bytes * 2 - 1).is_err());
    assert!(admit(&inputs, bytes).is_err());
    assert!(admit(&[(a, &mesh, [0; 5]), (a, &mesh, [0; 5])], bytes * 2).is_err());
    assert!(
        admit(
            &[(a, &mesh, [0; 5]), (b, &mesh, [0, 0, 0, 7, 0])],
            bytes * 2
        )
        .is_err()
    );
    let selected = mesh.with_uv_set(7, vec![[0.5, 0.5]; mesh.vertex_count()])?;
    admit(
        &[(a, &mesh, [0; 5]), (b, &selected, [0, 0, 0, 7, 0])],
        bytes * 2,
    )?;
    admit(&[], 0)?;
    Ok(())
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn source_replacement_bounds_cache_and_preserves_retained_geometry() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let mut graph = SceneGraph::new();
    let a = graph.insert(None, Node::new())?;
    let b = graph.insert(None, Node::new())?;
    let mesh = Mesh::plane();
    let mesh = mesh.with_uv_set(1, vec![[0.25, 0.75]; mesh.vertex_count()])?;
    let output = GpuDeformationOutput::upload(context.clone(), mesh.clone(), Default::default())?;
    let bytes =
        Scene3dGpuGeometryMemory::plan(mesh.vertex_count(), mesh.index_count())?.total_bytes;
    let original = prepare(
        &Sources::new(),
        &[(a, &output, [0; 5]), (b, &output, [0; 5])],
        bytes * 2,
    )?;
    let retained = output.render_geometry(&original[&a].1)?;
    let mut current = original.clone();
    for set in [1, 0, 1] {
        current = prepare(
            &current,
            &[(b, &output, [0; 5]), (a, &output, [set; 5])],
            bytes * 2,
        )?;
        assert_eq!(current.len(), 2);
        assert_eq!(current[&a].1.uv_sets(), [set; 5]);
        let packed = output.render_geometry(&current[&a].1)?;
        assert_eq!(packed.indices(), retained.indices());
        assert_eq!(original[&a].1.uv_sets(), [0; 5]);
    }
    let changed = mesh.with_vertex_colors(vec![[0.3, 0.4, 0.5, 1.]; mesh.vertex_count()])?;
    let changed = GpuDeformationOutput::upload(context, changed, GpuDeformationLimits::default())?;
    current = prepare(&current, &[(a, &changed, [1; 5])], bytes)?;
    assert_eq!(current.len(), 1);
    assert!(current[&a].0.ptr_eq(changed.base_mesh()));
    assert!(output.render_geometry(&current[&a].1).is_err());
    let packed = changed.render_geometry(&current[&a].1)?;
    assert_eq!(packed.indices(), retained.indices());
    current = prepare(&current, &[], 0)?;
    assert!(current.is_empty());
    drop(current);
    drop(changed);
    output.render_geometry(&original[&a].1)?;
    assert_eq!(retained.uv_sets(), [0; 5]);
    Ok(())
}
