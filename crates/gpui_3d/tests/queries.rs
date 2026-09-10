use gpui::{Bounds, Pixels, point, px, rgb, rgba, size};
use gpui_3d::{
    AffineTransform, Camera, Material, Mesh, Node, Object, ObjectId, PickBehavior, Projection, Ray,
    ReparentMode, Scene, SceneGraph,
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
