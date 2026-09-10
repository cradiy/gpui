use gpui::{Bounds, Pixels, point, px, rgb, rgba, size};
use gpui_3d::{
    Aabb, AffineTransform, Camera, Material, Mesh, Node, Object, ObjectId, PickBehavior,
    Projection, Ray, ReparentMode, Scene, SceneGraph,
};
use std::collections::HashSet;

fn plane() -> Object {
    Object::new(Mesh::plane(), Material::color(rgb(0xffffff)))
}

fn ray() -> Ray {
    Ray::new([0., 0., 6.], [0., 0., -1.]).unwrap()
}

fn viewport() -> Bounds<Pixels> {
    Bounds::new(point(px(30.), px(50.)), size(px(600.), px(400.)))
}

#[test]
fn bounds_candidates_are_camera_independent_and_do_not_apply_surface_policy() {
    let scene = Scene::new()
        .object(plane().id("ignored").pick_behavior(PickBehavior::Ignore))
        .object(Object::new(Mesh::plane(), Material::color(rgba(0xffffff00))).id("transparent"))
        .object(plane().id("occluder").pick_behavior(PickBehavior::Occlude))
        .object(plane().position([50., 0., 0.]));
    let region = Aabb::new([0.; 3], [0.; 3]).unwrap();
    let result = scene.bounds_candidates(region);
    assert_eq!(
        result
            .iter()
            .map(|object| object.object_index)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(result[0].pick_behavior, PickBehavior::Ignore);
    assert_eq!(result[1].object_id, Some(&ObjectId::from("transparent")));
    assert!(result.iter().all(|object| object.node.is_none()));
    let mut visited = Vec::new();
    let filtered = scene.bounds_candidates_where(region, |object| {
        visited.push(object.object_index);
        object.object_id == Some(&ObjectId::from("transparent"))
    });
    assert_eq!(visited, vec![0, 1, 2]);
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].object_index, 1);
    let other = scene.clone().camera(Camera {
        eye: [100., 100., 100.],
        target: [101., 100., 100.],
        ..Camera::default()
    });
    assert_eq!(other.bounds_candidates(region).len(), 3);
    assert!(scene.raycast(ray()).is_none());
    assert!(Scene::new().bounds_candidates(region).is_empty());
}

#[test]
fn conservative_bounds_do_not_claim_triangle_contact() {
    let mut graph = SceneGraph::new();
    graph
        .insert(
            None,
            Node::new()
                .mesh(Mesh::plane(), Material::color(rgb(0xffffff)))
                .transform(
                    AffineTransform::from_trs(
                        [0.; 3],
                        [
                            0.,
                            0.,
                            (std::f32::consts::FRAC_PI_8).sin(),
                            (std::f32::consts::FRAC_PI_8).cos(),
                        ],
                        [1.; 3],
                    )
                    .unwrap(),
                ),
        )
        .unwrap();
    let scene = graph.evaluate().unwrap().scene(Camera::default());
    let empty_corner = Aabb::new([0.6, 0.6, 0.], [0.6, 0.6, 0.]).unwrap();
    assert_eq!(scene.bounds_candidates(empty_corner).len(), 1);
    assert!(
        scene
            .raycast(Ray::new([0.6, 0.6, 1.], [0., 0., -1.]).unwrap())
            .is_none()
    );
    let outside = Aabb::new([0.8, 0.8, 0.], [0.8, 0.8, 0.]).unwrap();
    assert!(scene.bounds_candidates(outside).is_empty());
}

#[test]
fn bounds_refits_follow_visibility_order_and_deformation_without_changing_old_states() {
    let mut graph = SceneGraph::new();
    let material = Material::color(rgb(0xffffff));
    let a = graph
        .insert(None, Node::new().mesh(Mesh::plane(), material.clone()))
        .unwrap();
    let root = graph.insert(None, Node::new()).unwrap();
    let mesh = Mesh::plane();
    let b = graph
        .insert(
            Some(root),
            Node::new().id("moving").mesh(mesh.clone(), material),
        )
        .unwrap();
    let region = Aabb::new([-1.; 3], [1.; 3]).unwrap();
    let before = graph.evaluate().unwrap().scene(Camera::default());
    before.prepare_spatial_index();
    assert_eq!(
        before
            .bounds_candidates(region)
            .iter()
            .map(|object| object.node)
            .collect::<Vec<_>>(),
        vec![Some(a), Some(b)]
    );
    graph.set_visible(a, false).unwrap();
    graph.reparent(b, None, ReparentMode::KeepLocal).unwrap();
    let mut vertices = mesh.vertices().to_vec();
    for vertex in &mut vertices {
        vertex.position[0] += 10.;
    }
    graph
        .set_mesh(
            b,
            mesh.with_vertices(vertices, mesh.tangents().map(<[_]>::to_vec))
                .unwrap(),
        )
        .unwrap();
    let changed = graph.evaluate().unwrap().scene(Camera::default());
    changed.prepare_spatial_index_from(&before);
    assert!(changed.bounds_candidates(region).is_empty());
    let moved_region = Aabb::new([9., -1., -1.], [11., 1., 1.]).unwrap();
    let moved = changed.bounds_candidates(moved_region);
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].node, Some(b));
    assert_eq!(moved[0].object_index, 0);
    assert_eq!(moved[0].object_id, Some(&ObjectId::from("moving")));
    assert_eq!(before.bounds_candidates(region).len(), 2);
    assert!(before.bounds_candidates(moved_region).is_empty());
    graph.remove_subtree(b).unwrap();
    let removed = graph.evaluate().unwrap().scene(Camera::default());
    removed.prepare_spatial_index_from(&changed);
    assert!(removed.bounds_candidates(moved_region).is_empty());
    assert_eq!(changed.bounds_candidates(moved_region).len(), 1);
}

#[test]
fn bounds_filters_only_visit_overlapping_objects_in_original_order() {
    let mut scene = Scene::new();
    for i in (0..1024).rev() {
        scene = scene.object(plane().position([i as f32 * 4., 0., 0.]));
    }
    let region = Aabb::new([400., -0.1, 0.], [412., 0.1, 0.]).unwrap();
    let mut seen = Vec::new();
    let result = scene.bounds_candidates_where(region, |object| {
        seen.push(object.object_index);
        object.object_index % 2 == 0
    });
    assert_eq!(seen, vec![920, 921, 922, 923]);
    assert_eq!(
        result
            .iter()
            .map(|object| object.object_index)
            .collect::<Vec<_>>(),
        vec![920, 922]
    );
}

#[test]
fn excluded_occluders_do_not_block_targets_or_mutate_default_queries() {
    let scene = Scene::new().object(plane().id("back")).object(
        plane()
            .id("blocker")
            .position([0., 0., 1.])
            .pick_behavior(PickBehavior::Occlude),
    );
    let center = point(px(330.), px(250.));
    assert!(scene.raycast(ray()).is_none());
    assert!(scene.pick(viewport(), center).is_none());
    let target = ObjectId::from("back");
    let expected = Scene::new()
        .object(plane().id("back"))
        .raycast(ray())
        .unwrap();
    for hit in [
        scene.raycast_where(ray(), |object| object.object_id == Some(&target)),
        scene.pick_where(viewport(), center, |object| {
            object.object_id == Some(&target)
        }),
    ] {
        let hit = hit.unwrap();
        assert_eq!(hit.object_id, expected.object_id);
        assert_eq!(hit.object_index, 0);
        assert_eq!(hit.distance, expected.distance);
        assert_eq!(hit.position, expected.position);
        assert_eq!(hit.normal, expected.normal);
        assert_eq!(hit.uv, expected.uv);
        assert_eq!(hit.triangle_index, expected.triangle_index);
    }
    assert!(scene.raycast_where(ray(), |_| false).is_none());
    assert!(scene.raycast_where(ray(), |_| true).is_none());
    assert!(scene.raycast(ray()).is_none());
    assert!(scene.pick(viewport(), center).is_none());
}

#[test]
fn accepted_candidates_retain_pick_behavior_alpha_and_tie_order() {
    let scene = Scene::new()
        .object(
            plane()
                .id("ignored")
                .position([0., 0., 3.])
                .pick_behavior(PickBehavior::Ignore),
        )
        .object(
            Object::new(Mesh::plane(), Material::color(rgba(0xffffff00)))
                .id("transparent")
                .position([0., 0., 2.]),
        )
        .object(plane())
        .object(plane().id("coincident"));
    let mut seen = HashSet::new();
    let hit = scene
        .raycast_where(ray(), |object| {
            assert!(seen.insert(object.object_index), "candidate visited twice");
            if object.object_id == Some(&ObjectId::from("ignored")) {
                assert_eq!(object.pick_behavior, PickBehavior::Ignore);
            }
            true
        })
        .unwrap();
    assert_eq!(hit.object_index, 2);
    assert_eq!(hit.object_id, None);
    assert!(
        scene
            .raycast_where(ray(), |object| object.object_index < 2)
            .is_none()
    );
    let hit = scene
        .raycast_where(ray(), |object| object.object_id.is_some())
        .unwrap();
    assert_eq!(hit.object_index, 3);
    assert_eq!(hit.object_id, Some(ObjectId::from("coincident")));
}

#[test]
fn node_filters_follow_refitted_snapshots_without_relying_on_object_indices() {
    let mut graph = SceneGraph::new();
    let front = graph
        .insert(
            None,
            Node::new()
                .id("front")
                .mesh(Mesh::plane(), Material::color(rgb(0xffffff)))
                .transform(AffineTransform::from_translation([0., 0., 1.]).unwrap()),
        )
        .unwrap();
    let root = graph.insert(None, Node::new()).unwrap();
    let target = graph
        .insert(
            Some(root),
            Node::new()
                .id("target")
                .mesh(Mesh::plane(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let before = graph.evaluate().unwrap().scene(Camera::default());
    before.prepare_spatial_index();
    let first = before
        .raycast_where(ray(), |object| object.node == Some(target))
        .unwrap();
    graph.set_visible(front, false).unwrap();
    graph
        .reparent(target, None, ReparentMode::KeepWorld)
        .unwrap();
    graph
        .set_transform(
            target,
            AffineTransform::from_translation([0., 0., -2.]).unwrap(),
        )
        .unwrap();
    let after = graph.evaluate().unwrap().scene(Camera::default());
    after.prepare_spatial_index_from(&before);
    let second = after
        .raycast_where(ray(), |object| object.node == Some(target))
        .unwrap();
    assert_eq!(second.node, first.node);
    assert_eq!(second.object_id, first.object_id);
    assert_ne!(second.object_index, first.object_index);
    assert_eq!(second.position, [0., 0., -2.]);
    assert_eq!(second.distance, 8.);
    assert_eq!(
        before
            .raycast_where(ray(), |object| object.node == Some(target))
            .unwrap()
            .position,
        first.position
    );
    assert!(
        after
            .raycast_where(ray(), |object| object.node == Some(front))
            .is_none()
    );
}

#[test]
fn screen_filters_keep_projection_clipping_separate_from_world_rays() {
    for projection in [
        Camera::default().projection,
        Projection::Orthographic { vertical_size: 4. },
    ] {
        let camera = Camera {
            near: 1.,
            far: 5.,
            projection,
            ..Camera::default()
        };
        let scene = Scene::new()
            .camera(camera)
            .object(plane().id("near").position([0., 0., 5.5]))
            .object(plane().id("inside").position([0., 0., 3.]))
            .object(plane().id("far"));
        let center = point(px(330.), px(250.));
        let inside = scene.pick_where(viewport(), center, |_| true).unwrap();
        assert_eq!(inside.object_id, Some(ObjectId::from("inside")));
        let far = ObjectId::from("far");
        assert!(
            scene
                .pick_where(viewport(), center, |object| object.object_id == Some(&far))
                .is_none()
        );
        assert_eq!(
            scene
                .raycast_where(ray(), |object| object.object_id == Some(&far))
                .unwrap()
                .distance,
            6.
        );
        assert_eq!(scene.raycast_where(ray(), |_| true).unwrap().distance, 0.5);
        for position in [point(px(0.), px(0.)), point(px(630.), px(250.))] {
            assert!(
                scene
                    .pick_where(viewport(), position, |_| panic!(
                        "outside viewport must not traverse candidates"
                    ))
                    .is_none()
            );
        }
    }
}

#[test]
fn filtering_reuses_bvh_candidate_pruning() {
    let mut scene = Scene::new().object(plane().id("center"));
    for i in 0..1024 {
        scene = scene.object(plane().position([100. + i as f32 * 2., 0., 0.]));
    }
    scene.prepare_spatial_index();
    let mut candidates = HashSet::new();
    let hit = scene
        .raycast_where(ray(), |object| {
            assert!(candidates.insert(object.object_index));
            object.object_id.is_some()
        })
        .unwrap();
    assert_eq!(hit.object_id, Some(ObjectId::from("center")));
    assert!(
        candidates.len() < 32,
        "unrelated objects must be pruned before filtering"
    );
}
