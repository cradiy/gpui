use super::*;
use crate::{Aabb, Node, PreparationCache, SceneGraph, TextureState};
use gpui::{MeshTexture3d, rgb};

#[test]
fn object_updates_retain_old_frames_and_rebuild_graph_query_bounds() {
    let mesh = Mesh::cube();
    let mut graph = SceneGraph::new();
    let node = graph
        .insert(
            None,
            Node::new()
                .id("part")
                .mesh(mesh.clone(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let source = graph.evaluate().unwrap().scene(Camera::default());
    source.prepare_spatial_index();
    let mut cache = PreparationCache::with_capacity(2);
    let resolve = |_: crate::TextureRequest<'_>| Ok(TextureState::Ready(MeshTexture3d::None));
    let before = cache.prepare(&source, 1., None, resolve).unwrap();
    let replacement = Mesh::plane();
    let updated = source
        .with_object_updates([(
            1,
            ObjectUpdate::new()
                .world(AffineTransform::from_translation([1., 0., 0.]).unwrap())
                .mesh(replacement.clone())
                .material(Material::color(rgb(0xff0000))),
        )])
        .unwrap();
    let after = cache.prepare(&updated, 1., None, resolve).unwrap();
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(
        &before,
        &cache.prepare(&source, 1., None, resolve).unwrap()
    ));
    assert_eq!(before.frame().objects[0].color, rgb(0xffffff));
    assert_eq!(after.frame().objects[0].color, rgb(0xff0000));
    assert_eq!(after.frame().objects[0].model[3][0], 1.);
    assert!(Arc::ptr_eq(&before.frame().objects[0].mesh, &mesh.0));
    assert!(Arc::ptr_eq(&after.frame().objects[0].mesh, &replacement.0));
    assert_eq!(after.object(1).unwrap().node, Some(node));
    assert_eq!(after.object(1).unwrap().id, Some("part".into()));
    let region = Aabb::new([0.9, -0.1, -0.1], [1.1, 0.1, 0.1]).unwrap();
    assert!(source.bounds_candidates(region).is_empty());
    assert_eq!(updated.bounds_candidates(region).len(), 1);
    assert_eq!(
        graph.node(node).unwrap().surface().unwrap().1.color,
        rgb(0xffffff)
    );
}

#[test]
fn object_updates_validate_the_complete_combination_before_publication() {
    let source = Scene::new()
        .object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))))
        .object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))).position([100., 0., 0.]));
    let revision = source.preparation_revision.clone();
    let material = Material::image("surface.png").image_uv_set(1);
    let mesh = Mesh::cube();
    let uv_mesh = mesh
        .with_uv_set(1, mesh.vertices().iter().map(|v| v.uv).collect())
        .unwrap();
    let updated = source
        .with_object_updates([(
            2,
            ObjectUpdate::new().material(material.clone()).mesh(uv_mesh),
        )])
        .unwrap();
    assert_eq!(
        updated.geometry_inputs().unwrap().nth(1).unwrap().uv_sets[0],
        1
    );
    assert!(
        source
            .with_object_updates([
                (
                    1,
                    ObjectUpdate::new().material(Material::color(rgb(0xff0000)))
                ),
                (2, ObjectUpdate::new().material(material)),
            ])
            .is_err()
    );
    for targets in [[0, 1], [1, 3], [1, 1]] {
        assert!(
            source
                .with_object_updates(targets.map(|id| (id, ObjectUpdate::new())))
                .is_err()
        );
    }
    assert!(Arc::ptr_eq(&revision, &source.preparation_revision));
    assert_eq!(source.objects[0].material.color, rgb(0xffffff));
    assert_eq!(source.objects[1].material.uv_set, 0);
    let unchanged = source.with_object_updates([]).unwrap();
    assert!(Arc::ptr_eq(&revision, &unchanged.preparation_revision));
}

#[test]
fn cpu_mesh_replacement_clears_gpu_geometry_and_render_bounds_together() {
    let mut source = Scene::new().object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))));
    source.objects[0].gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(Arc::new(())));
    source.objects[0].render_bounds = Aabb::new([10.; 3], [11.; 3]);
    let result = source
        .with_object_updates([(1, ObjectUpdate::new().mesh(Mesh::plane()))])
        .unwrap();
    assert!(result.objects[0].gpu_geometry.is_none());
    assert!(result.objects[0].render_bounds.is_none());
    assert!(source.objects[0].gpu_geometry.is_some());
    assert!(source.objects[0].render_bounds.is_some());
    assert_eq!(result.plan_frame(1., None).unwrap().objects.len(), 1);
}

#[test]
fn material_batches_reuse_spatial_indices_but_apply_current_coverage() {
    let mut graph = SceneGraph::new();
    let front = graph
        .insert(
            None,
            Node::new().mesh(Mesh::plane(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    graph
        .insert(
            None,
            Node::new()
                .mesh(Mesh::plane(), Material::color(rgb(0xffffff)))
                .transform(AffineTransform::from_translation([0., 0., -1.]).unwrap()),
        )
        .unwrap();
    let source = graph.evaluate().unwrap().scene(Camera::default());
    source.prepare_spatial_index();
    let ray = crate::Ray::new([0.1, 0., 2.], [0., 0., -1.]).unwrap();
    let original_hit = source.raycast(ray).unwrap();
    assert_eq!(original_hit.node, Some(front));
    let mut cache = PreparationCache::with_capacity(2);
    let resolve = |_: crate::TextureRequest<'_>| Ok(TextureState::Ready(MeshTexture3d::None));
    let original_frame = cache.prepare(&source, 1., None, resolve).unwrap();
    let changed = source
        .with_object_updates([(
            original_hit.object_index as u32 + 1,
            ObjectUpdate::new().material(
                Material::color(gpui::rgba(0xffffff00)).alpha_mode(crate::AlphaMode::Blend),
            ),
        )])
        .unwrap();
    assert!(Arc::ptr_eq(&source.spatial_index, &changed.spatial_index));
    assert!(Arc::ptr_eq(
        source.spatial_source.as_ref().unwrap(),
        changed.spatial_source.as_ref().unwrap()
    ));
    let hit = changed.raycast(ray).unwrap();
    assert_ne!(hit.node, Some(front));
    assert!((hit.position[2] + 1.).abs() < 1e-6);
    assert_eq!(source.raycast(ray).unwrap().node, Some(front));
    let changed_frame = cache.prepare(&changed, 1., None, resolve).unwrap();
    assert!(!Arc::ptr_eq(&original_frame, &changed_frame));
    assert_eq!(original_frame.frame().objects[0].color.a, 1.);
    assert!(
        changed_frame
            .frame()
            .objects
            .iter()
            .any(|object| object.color.a == 0.)
    );
}
