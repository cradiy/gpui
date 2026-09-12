use super::*;
use crate::{PreparationCache, Ray, TextureState, Vertex};
use gpui::{MeshTexture3d, rgb};

fn triangle(x: f32, z: f32) -> Mesh {
    Mesh::new(
        [[x, 0., z], [x + 2., 0., z], [x, 2., z]]
            .into_iter()
            .zip([[0., 0.], [1., 0.], [0., 1.]])
            .map(|(position, uv)| Vertex {
                position,
                normal: [0., 0., 1.],
                uv,
            })
            .collect(),
        vec![0, 1, 2],
    )
}

#[test]
fn evaluated_mesh_replacement_preserves_constraints_and_updates_visible_and_hidden_bounds() {
    use crate::TransformConstraint;
    let mut graph = SceneGraph::new();
    let target = graph
        .insert(
            None,
            Node::new().transform(AffineTransform::from_translation([3., 0., 0.]).unwrap()),
        )
        .unwrap();
    let root = graph
        .insert(
            None,
            Node::new()
                .camera(Camera::default())
                .light(PunctualLight::directional([0., -1., 0.])),
        )
        .unwrap();
    let visible = graph
        .insert(
            Some(root),
            Node::new()
                .id("surface")
                .mesh(triangle(0., 0.), Material::color(rgb(0xaabbcc)))
                .cast_shadows(false)
                .receive_shadows(false),
        )
        .unwrap();
    let hidden_parent = graph
        .insert(Some(root), Node::new().visible(false))
        .unwrap();
    let hidden = graph
        .insert(
            Some(hidden_parent),
            Node::new().mesh(triangle(20., 0.), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let constraints = [(
        root,
        TransformConstraint::Follow {
            target,
            offset: AffineTransform::IDENTITY,
        },
    )];
    let old = graph.evaluate_with_constraints([], constraints).unwrap();
    old.prepare_spatial_index();
    let replacement = triangle(4., 2.)
        .with_vertex_colors(vec![[0.5, 0.25, 1., 1.]; 3])
        .unwrap();
    let meshes = [(visible, replacement.clone()), (hidden, triangle(40., 5.))];
    let expected = graph
        .evaluate_with_constraints_and_meshes([], constraints, meshes.clone())
        .unwrap();
    let current = old.with_meshes(meshes).unwrap();
    current.prepare_spatial_index_from(&old);
    for node in current.nodes() {
        let original = old.node(node.handle).unwrap();
        let reference = expected.node(node.handle).unwrap();
        assert_eq!(node.world, original.world);
        assert_eq!(node.parent, original.parent);
        assert_eq!(node.id, original.id);
        assert_eq!(node.visible, original.visible);
        assert_eq!(node.camera, original.camera);
        assert_eq!(
            node.light
                .map(|light| (light.kind(), light.position(), light.direction())),
            original
                .light
                .map(|light| (light.kind(), light.position(), light.direction()))
        );
        assert_eq!(node.bounds, reference.bounds);
        assert_eq!(node.subtree_bounds, reference.subtree_bounds);
        assert_eq!(
            current.constraint_status(node.handle),
            old.constraint_status(node.handle)
        );
    }
    assert_eq!(
        current.bounds(),
        Some(Aabb::new([7., 0., 2.], [9., 2., 2.]).unwrap())
    );
    assert_eq!(
        current.node(root).unwrap().subtree_bounds.unwrap().max(),
        [45., 2., 5.]
    );
    assert_eq!(
        current
            .lights()
            .map(|(handle, light)| (handle, light.kind(), light.direction()))
            .collect::<Vec<_>>(),
        old.lights()
            .map(|(handle, light)| (handle, light.kind(), light.direction()))
            .collect::<Vec<_>>()
    );
    assert_eq!(current.revision(), old.revision());
    assert_eq!(graph.revision(), old.revision());
    let scene = current.scene(Camera::orbit(0., 0., 30.));
    let ray = Ray::new([7.25, 0.25, 10.], [0., 0., -1.]).unwrap();
    let hit = scene.raycast(ray).unwrap();
    assert_eq!(hit.node, Some(visible));
    assert_eq!(hit.position[2], 2.);
    assert!(old.scene(Camera::default()).raycast(ray).is_none());
    assert!(
        scene
            .bounds_candidates(current.node(hidden).unwrap().bounds.unwrap())
            .is_empty()
    );
    assert_eq!(scene.objects.len(), 1);
    assert!(!scene.objects[0].cast_shadows && !scene.objects[0].receive_shadows);
    assert_eq!(
        scene.objects[0].material.color,
        old.objects[0].material.color
    );
    let mut cache = PreparationCache::new();
    let resolve = |_: crate::TextureRequest<'_>| Ok(TextureState::Ready(MeshTexture3d::None));
    let a = cache
        .prepare(&old.scene(scene.camera), 1., None, resolve)
        .unwrap();
    let b = cache.prepare(&scene, 1., None, resolve).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(Arc::ptr_eq(&b.frame().objects[0].mesh, &replacement.0));
    assert!(a.frame().objects[0].mesh.vertex_colors().is_none());
    let empty = current.with_meshes([]).unwrap();
    assert!(Arc::ptr_eq(
        &empty.preparation_revision,
        &current.preparation_revision
    ));
    assert!(Arc::ptr_eq(&empty.spatial_index, &current.spatial_index));
    graph.remove_subtree(root).unwrap();
    drop(graph);
    let restored = current
        .with_meshes([(visible, triangle(0., 0.)), (hidden, triangle(20., 0.))])
        .unwrap();
    restored.prepare_spatial_index_from(&current);
    assert_eq!(restored.bounds(), old.bounds());
    assert_eq!(
        restored.node(root).unwrap().subtree_bounds,
        old.node(root).unwrap().subtree_bounds
    );
    assert!(restored.scene(Camera::default()).raycast(ray).is_none());
    assert!(current.scene(Camera::default()).raycast(ray).is_some());
}

#[test]
fn evaluated_mesh_replacement_rejects_invalid_targets_without_changing_snapshot() {
    let mut graph = SceneGraph::new();
    let group = graph.insert(None, Node::new()).unwrap();
    let mesh = graph
        .insert(
            None,
            Node::new()
                .mesh(triangle(0., 0.), Material::color(rgb(0xffffff)))
                .transform(AffineTransform::from_translation([f32::MAX, 0., 0.]).unwrap()),
        )
        .unwrap();
    let expired = graph.insert(None, Node::new()).unwrap();
    graph.remove_subtree(expired).unwrap();
    let foreign = SceneGraph::new().insert(None, Node::new()).unwrap();
    let snapshot = graph.evaluate().unwrap();
    let old_bounds = snapshot.bounds();
    for invalid in [expired, foreign] {
        assert!(
            matches!(snapshot.with_meshes([(mesh, triangle(4., 0.)), (invalid, triangle(0., 0.))]),
            Err(SceneError::InvalidHandle(handle)) if handle == invalid)
        );
    }
    assert!(
        matches!(snapshot.with_meshes([(group, triangle(0., 0.))]), Err(SceneError::NoMesh(handle)) if handle == group)
    );
    assert!(
        matches!(snapshot.with_meshes([(mesh, triangle(0., 0.)), (mesh, triangle(4., 0.))]), Err(SceneError::DuplicateMesh(handle)) if handle == mesh)
    );
    assert!(
        matches!(snapshot.with_meshes([(mesh, triangle(f32::MAX, 0.))]), Err(SceneError::InvalidTransform { node, .. }) if node == mesh)
    );
    assert_eq!(snapshot.bounds(), old_bounds);
    assert!(Arc::ptr_eq(
        &snapshot.objects[0].mesh.0,
        &graph.node(mesh).unwrap().surface().unwrap().0.0
    ));
}

#[test]
fn mesh_and_transform_overrides_share_bounds_picking_and_preserve_source() {
    let base = triangle(0., 0.);
    let replacement = triangle(4., 2.)
        .with_uv_set(2, vec![[0.5, 0.25]; 3])
        .unwrap()
        .with_vertex_colors(vec![[0.2, 0.4, 0.6, 1.]; 3])
        .unwrap();
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let first = graph
        .insert(
            Some(root),
            Node::new()
                .id("first")
                .mesh(base.clone(), Material::color(rgb(0xabcdef)))
                .cast_shadows(false)
                .receive_shadows(false),
        )
        .unwrap();
    let second = graph
        .insert(
            Some(root),
            Node::new()
                .mesh(base.clone(), Material::color(rgb(0xffffff)))
                .transform(AffineTransform::from_translation([20., 0., 0.]).unwrap()),
        )
        .unwrap();
    let revision = graph.revision();
    let old = graph.evaluate().unwrap();
    old.prepare_spatial_index();
    let current = graph
        .evaluate_with_overrides(
            [(
                root,
                AffineTransform::from_translation([1., 0., 0.]).unwrap(),
            )],
            [(first, replacement.clone())],
        )
        .unwrap();
    current.prepare_spatial_index_from(&old);
    let scene = current.scene(Camera::default());
    let ray = Ray::new([5.25, 0.25, 10.], [0., 0., -1.]).unwrap();
    let hit = scene.raycast(ray).unwrap();
    assert_eq!(hit.node, Some(first));
    assert_eq!(hit.object_id, Some("first".into()));
    assert_eq!(hit.position, [5.25, 0.25, 2.]);
    assert!(old.scene(Camera::default()).raycast(ray).is_none());
    assert_eq!(
        current.node(first).unwrap().bounds,
        Some(Aabb::new([5., 0., 2.], [7., 2., 2.]).unwrap())
    );
    assert_eq!(current.node(root).unwrap().subtree_bounds, current.bounds());
    let sibling = scene
        .raycast(Ray::new([21.25, 0.25, 10.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(sibling.node, Some(second));
    assert_eq!(sibling.position[2], 0.);
    let object = &scene.objects[0];
    assert!(Arc::ptr_eq(&object.mesh.0, &replacement.0));
    assert!(!object.cast_shadows && !object.receive_shadows);
    assert_eq!(object.material.color, old.objects[0].material.color);
    assert_eq!(object.mesh.uv_at(2, 0), Some([0.5, 0.25]));
    assert_eq!(object.mesh.vertex_colors(), replacement.vertex_colors());
    assert!(Arc::ptr_eq(&scene.objects[1].mesh.0, &base.0));
    assert!(Arc::ptr_eq(&old.objects[0].mesh.0, &base.0));
    assert!(Arc::ptr_eq(
        &graph.node(first).unwrap().surface.as_ref().unwrap().0.0,
        &base.0
    ));
    assert_eq!(
        graph.node(root).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
    assert_eq!(graph.revision(), revision);
    assert_eq!(current.revision(), old.revision());
    assert_eq!(graph.evaluate().unwrap().bounds(), old.bounds());

    graph.set_visible(first, false).unwrap();
    let hidden = graph
        .evaluate_with_overrides([], [(first, replacement.clone())])
        .unwrap();
    hidden.prepare_spatial_index_from(&current);
    assert_eq!(
        hidden.node(first).unwrap().bounds,
        Some(replacement.bounds())
    );
    assert_eq!(hidden.scene(Camera::default()).objects.len(), 1);
    assert!(hidden.scene(Camera::default()).raycast(ray).is_none());
    assert_eq!(hidden.bounds(), hidden.node(second).unwrap().bounds);
}

#[test]
fn mesh_override_snapshots_invalidate_preparation_without_graph_edits() {
    let mut graph = SceneGraph::new();
    let node = graph
        .insert(
            None,
            Node::new().mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let old = graph.evaluate().unwrap();
    let replacement = Mesh::cube()
        .with_vertex_colors(vec![[0.5, 0.25, 1., 1.]; Mesh::cube().vertices().len()])
        .unwrap();
    let current = graph
        .evaluate_with_overrides([], [(node, replacement.clone())])
        .unwrap();
    let mut cache = PreparationCache::new();
    let resolve = |_: crate::TextureRequest<'_>| Ok(TextureState::Ready(MeshTexture3d::None));
    let old_scene = old.scene(Camera::default());
    let current_scene = current.scene(Camera::default());
    let a = cache.prepare(&old_scene, 1., None, resolve).unwrap();
    let b = cache.prepare(&old_scene, 1., None, resolve).unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let c = cache.prepare(&current_scene, 1., None, resolve).unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(c.frame().objects.len(), 1);
    assert!(Arc::ptr_eq(&c.frame().objects[0].mesh, &replacement.0));
    assert!(a.frame().objects[0].mesh.vertex_colors().is_none());
    assert_eq!(old.revision(), current.revision());
}

#[test]
fn override_errors_leave_authored_graph_and_snapshots_unchanged() {
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let node = graph
        .insert(
            Some(root),
            Node::new().mesh(triangle(0., 0.), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let expired = graph.insert(None, Node::new()).unwrap();
    graph.remove_subtree(expired).unwrap();
    let foreign = SceneGraph::new().insert(None, Node::new()).unwrap();
    let old = graph.evaluate().unwrap();
    let revision = graph.revision();
    let mesh = triangle(4., 2.);
    assert!(
        matches!(graph.evaluate_with_overrides([], [(node, mesh.clone()), (node, mesh.clone())]), Err(SceneError::DuplicateMesh(n)) if n == node)
    );
    assert!(
        matches!(graph.evaluate_with_overrides([(root, AffineTransform::IDENTITY); 2], [(node, mesh.clone())]), Err(SceneError::DuplicateTransform(n)) if n == root)
    );
    assert!(
        matches!(graph.evaluate_with_overrides([], [(root, mesh.clone())]), Err(SceneError::NoMesh(n)) if n == root)
    );
    for handle in [expired, foreign] {
        assert!(
            matches!(graph.evaluate_with_overrides([], [(node, mesh.clone()), (handle, mesh.clone())]), Err(SceneError::InvalidHandle(n)) if n == handle)
        );
    }
    let huge = AffineTransform::from_translation([f32::MAX, 0., 0.]).unwrap();
    assert!(
        matches!(graph.evaluate_with_overrides([(root, huge), (node, huge)], [(node, mesh)]), Err(SceneError::InvalidTransform { node: n, .. }) if n == node)
    );
    assert_eq!(graph.revision(), revision);
    assert_eq!(
        graph.node(node).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
    assert_eq!(graph.evaluate().unwrap().bounds(), old.bounds());
    let ray = Ray::new([0.25, 0.25, 10.], [0., 0., -1.]).unwrap();
    assert_eq!(
        old.scene(Camera::default()).raycast(ray).unwrap().node,
        Some(node)
    );
    assert_eq!(
        graph
            .evaluate()
            .unwrap()
            .scene(Camera::default())
            .raycast(ray)
            .unwrap()
            .node,
        Some(node)
    );
}
