use gpui::rgb;
use gpui_3d::{
    AffineTransform, AimSettings, Camera, ConstraintStatus, Material, Mesh, MorphTarget,
    MorphTargets, Node, Ray, ResolvedTexture, SceneError, SceneGraph, Skin, SkinInfluence,
    TextureState, TransformConstraint,
};

fn translate(position: [f32; 3]) -> AffineTransform {
    AffineTransform::from_translation(position).unwrap()
}

#[test]
fn constrained_morph_skin_samples_preserve_world_geometry_and_outputs() {
    let base = Mesh::plane();
    let morph = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(vec![[1., 0., 0.]; base.vertex_count()].into()),
            ..Default::default()
        }],
    )
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        (0..base.vertex_count()).map(|_| {
            [SkinInfluence {
                joint: 0,
                weight: 1.,
            }]
        }),
    )
    .unwrap();
    let mut graph = SceneGraph::new();
    let driver = graph.insert(None, Node::new().visible(false)).unwrap();
    let joint = graph.insert(None, Node::new()).unwrap();
    let surface = graph
        .insert(
            None,
            Node::new()
                .id("surface")
                .mesh(base, Material::color(rgb(0x80c0e0)))
                .transform(translate([3., 0., 0.])),
        )
        .unwrap();
    let constraints = [
        (
            surface,
            TransformConstraint::Aim {
                target: driver,
                target_offset: [0.; 3],
                settings: AimSettings::default(),
            },
        ),
        (
            joint,
            TransformConstraint::Follow {
                target: driver,
                offset: AffineTransform::IDENTITY,
            },
        ),
    ];
    let revision = graph.revision();
    let authored = graph.evaluate().unwrap();
    let mut previous = authored.clone();
    previous.prepare_spatial_index();
    for x in [2., 0., 1., 2.] {
        let transforms = [(driver, translate([x, 0.5, -2.]))];
        let poses = graph
            .evaluate_with_constraints(transforms, constraints)
            .unwrap();
        let mesh_world = poses.node(surface).unwrap().world;
        let deformed = skin
            .evaluate_world(
                &morph.evaluate(&[0.25]).unwrap(),
                mesh_world,
                &[poses.node(joint).unwrap().world],
            )
            .unwrap();
        let sample = graph
            .evaluate_with_constraints_and_meshes(
                transforms,
                constraints,
                [(surface, deformed.clone())],
            )
            .unwrap();
        sample.prepare_spatial_index_from(&previous);
        assert_eq!(
            sample.constraint_status(joint),
            Some(ConstraintStatus::Follow)
        );
        assert_eq!(
            sample.constraint_status(surface),
            poses.constraint_status(surface)
        );
        assert_eq!(sample.node(surface).unwrap().world, mesh_world);
        assert_eq!(
            sample.node(surface).unwrap().bounds,
            Some(deformed.bounds().transformed(mesh_world).unwrap())
        );
        let scene = sample.scene(Camera {
            eye: [0., 0., 10.],
            target: [0., 0., -2.],
            ..Default::default()
        });
        let ray = Ray::new([x + 0.25, 0.5, 3.], [0., 0., -1.]).unwrap();
        let hit = scene.raycast(ray).unwrap();
        assert_eq!(hit.node, Some(surface));
        assert_eq!(hit.object_id, Some("surface".into()));
        for (actual, expected) in hit.position.into_iter().zip([x + 0.25, 0.5, -2.]) {
            assert!((actual - expected).abs() < 1e-5);
        }
        let prepared = scene
            .prepare(1., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
            .unwrap();
        let draw = &prepared.frame().objects[0];
        assert_eq!(draw.model, mesh_world.matrix());
        assert!(std::ptr::eq(draw.mesh.vertices(), deformed.vertices()));
        assert_eq!(prepared.objects()[0].node, Some(surface));
        assert_eq!(graph.revision(), revision);
        assert_eq!(sample.revision(), revision);
        assert!(authored.scene(Camera::default()).raycast(ray).is_none());
        previous = sample;
    }
    assert_eq!(graph.evaluate().unwrap().bounds(), authored.bounds());
    let plain = graph
        .evaluate_with_constraints_and_meshes([], [], [(surface, Mesh::cube())])
        .unwrap();
    assert_eq!(plain.constraint_status(surface), None);
    assert_eq!(
        plain.node(surface).unwrap().world,
        authored.node(surface).unwrap().world
    );
    assert_ne!(plain.bounds(), authored.bounds());
}

#[test]
fn constrained_mesh_validation_and_cycles_leave_prior_results_usable() {
    let mut graph = SceneGraph::new();
    let target = graph
        .insert(None, Node::new().transform(translate([2., 0., 0.])))
        .unwrap();
    let surface = graph
        .insert(
            None,
            Node::new().mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
        )
        .unwrap();
    let expired = graph.insert(None, Node::new()).unwrap();
    graph.remove_subtree(expired).unwrap();
    let foreign = SceneGraph::new().insert(None, Node::new()).unwrap();
    let binding = (
        surface,
        TransformConstraint::Follow {
            target,
            offset: AffineTransform::IDENTITY,
        },
    );
    let old = graph
        .evaluate_with_constraints_and_meshes([], [binding], [(surface, Mesh::plane())])
        .unwrap();
    let revision = graph.revision();
    for constraints in [vec![], vec![binding]] {
        for invalid in [expired, foreign] {
            assert!(
                matches!(graph.evaluate_with_constraints_and_meshes([], constraints.clone(), [(invalid, Mesh::cube())]),
                Err(SceneError::InvalidHandle(n)) if n == invalid)
            );
        }
        assert!(
            matches!(graph.evaluate_with_constraints_and_meshes([], constraints.clone(), [(target, Mesh::cube())]),
            Err(SceneError::NoMesh(n)) if n == target)
        );
        assert!(
            matches!(graph.evaluate_with_constraints_and_meshes([], constraints, [(surface, Mesh::cube()), (surface, Mesh::plane())]),
            Err(SceneError::DuplicateMesh(n)) if n == surface)
        );
    }
    let cycle = [
        binding,
        (
            target,
            TransformConstraint::Follow {
                target: surface,
                offset: AffineTransform::IDENTITY,
            },
        ),
    ];
    assert!(matches!(
        graph.evaluate_with_constraints_and_meshes([], cycle, [(surface, Mesh::plane())]),
        Err(SceneError::ConstraintCycle(_))
    ));
    assert!(
        matches!(graph.evaluate_with_constraints_and_meshes([], [binding; 2], [(surface, Mesh::plane())]),
        Err(SceneError::DuplicateConstraint(n)) if n == surface)
    );
    assert_eq!(graph.revision(), revision);
    assert_eq!(
        old.constraint_status(surface),
        Some(ConstraintStatus::Follow)
    );
    let hit = old
        .scene(Camera::default())
        .raycast(Ray::new([2., 0., 3.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(hit.node, Some(surface));
    assert_eq!(hit.position, [2., 0., 0.]);
    assert_eq!(
        graph.node(surface).unwrap().local_transform(),
        AffineTransform::IDENTITY
    );
}
