use gpui_3d::{
    Camera, Material, Mesh, MeshError, MeshUpdateError, MorphAttribute, MorphError, MorphTarget,
    MorphTargets, Node, Object, Ray, Scene, SceneError, SceneGraph, Vertex, VertexAttribute,
};

fn vertices() -> Vec<Vertex> {
    [[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]]
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
        })
        .to_vec()
}

fn rejects(vertices: Vec<Vertex>, indices: Vec<u32>, expected: MeshError) {
    assert_eq!(
        Mesh::try_new(vertices.clone(), indices.clone()).unwrap_err(),
        expected
    );
    assert_eq!(
        gpui::Mesh3d::try_new(vertices, indices).unwrap_err(),
        expected
    );
}

#[test]
fn malformed_meshes_report_actionable_locations() {
    rejects(vec![], vec![], MeshError::EmptyVertices);
    rejects(vertices(), vec![], MeshError::EmptyIndices);
    rejects(
        vertices(),
        vec![0, 1, 2, 0],
        MeshError::IncompleteTriangle { index_count: 4 },
    );
    for index in [3, u32::MAX] {
        rejects(
            vertices(),
            vec![0, index, 2],
            MeshError::IndexOutOfBounds {
                offset: 1,
                index,
                vertex_count: 3,
            },
        );
    }
    for (attribute, components) in [
        (VertexAttribute::Position, 3),
        (VertexAttribute::Normal, 3),
        (VertexAttribute::Uv, 2),
    ] {
        for component in 0..components {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut data = vertices();
                let mut unused = data[0];
                let values: &mut [f32] = match attribute {
                    VertexAttribute::Position => &mut unused.position,
                    VertexAttribute::Normal => &mut unused.normal,
                    VertexAttribute::Uv => &mut unused.uv,
                };
                values[component] = value;
                data.push(unused);
                rejects(
                    data,
                    vec![0, 1, 2],
                    MeshError::NonFiniteVertex {
                        vertex: 3,
                        attribute,
                        component,
                    },
                );
            }
        }
    }
}

#[test]
fn inspection_shares_storage_and_preserves_triangle_identity() {
    let mut data = vertices();
    data.push(Vertex {
        position: [8., 4., -2.],
        normal: [0.; 3],
        uv: [-2., 3.],
    });
    let indices = vec![0, 0, 0, 0, 1, 2];
    let mesh = Mesh::try_new(data.clone(), indices.clone()).unwrap();
    let copy = mesh.clone();
    assert!(std::ptr::eq(mesh.vertices(), copy.vertices()));
    assert!(std::ptr::eq(mesh.indices(), copy.indices()));
    assert_eq!(mesh.vertex_count(), data.len());
    assert_eq!(mesh.index_count(), indices.len());
    assert_eq!(mesh.triangle_count(), 2);
    assert_eq!(mesh.indices(), indices);
    for (actual, expected) in mesh.vertices().iter().zip(&data) {
        assert_eq!(actual.position, expected.position);
        assert_eq!(actual.normal, expected.normal);
        assert_eq!(actual.uv, expected.uv);
    }
    assert_eq!(mesh.bounds().min(), [-1., -1., -2.]);
    assert_eq!(mesh.bounds().max(), [8., 4., 0.]);
    let scene = Scene::new().object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))));
    let ray = Ray::new([0., 0., 1.], [0., 0., -1.]).unwrap();
    let hit = scene.raycast(ray).unwrap();
    assert_eq!(hit.triangle_index, 1);
    assert_eq!(hit.position, [0.; 3]);
    let degenerate = Mesh::try_new(data, vec![0, 0, 0]).unwrap();
    assert!(
        Scene::new()
            .object(Object::new(
                degenerate,
                Material::color(gpui::rgb(0xffffff))
            ))
            .raycast(ray)
            .is_none()
    );
    drop(scene);
    assert_eq!(copy.indices(), indices);
}

#[test]
fn vertex_snapshots_preserve_topology_and_refresh_bounds_queries_and_nodes() {
    let original = Mesh::plane();
    original.prepare_spatial_index();
    let mut vertices = original.vertices().to_vec();
    for vertex in &mut vertices {
        vertex.position[0] += 4.;
        vertex.position[2] = 0.25;
        vertex.uv = [0.3, 0.7];
        vertex.normal = [0.2, 0., 1.];
    }
    let moved = original
        .with_vertices(vertices, original.tangents().map(<[_]>::to_vec))
        .unwrap();
    assert!(std::ptr::eq(original.indices(), moved.indices()));
    assert!(!std::ptr::eq(original.vertices(), moved.vertices()));
    assert_eq!(moved.bounds().min(), [3.5, -0.5, 0.25]);
    let n = moved.vertices()[0].normal;
    let t = moved.tangents().unwrap()[0];
    assert!((n[0] * t[0] + n[1] * t[1] + n[2] * t[2]).abs() < 1e-6);
    let mut graph = SceneGraph::new();
    let root = graph.insert(None, Node::new()).unwrap();
    let node = graph
        .insert(
            Some(root),
            Node::new()
                .id("mesh")
                .mesh(original.clone(), Material::color(gpui::rgb(0xffffff))),
        )
        .unwrap();
    let before = graph.evaluate().unwrap();
    before.prepare_spatial_index();
    let revision = graph.revision();
    assert!(
        matches!(graph.set_mesh(root, moved.clone()), Err(SceneError::NoMesh(handle)) if handle == root)
    );
    assert_eq!(graph.revision(), revision);
    graph.set_mesh(node, moved.clone()).unwrap();
    let after = graph.evaluate().unwrap();
    assert_eq!(graph.revision(), revision + 1);
    assert_eq!(graph.parent(node).unwrap(), Some(root));
    assert_eq!(after.bounds(), Some(moved.bounds()));
    assert_eq!(after.node(root).unwrap().subtree_bounds, after.bounds());
    let scene = after.scene(Camera::default());
    let old_ray = Ray::new([0., 0., 2.], [0., 0., -1.]).unwrap();
    let new_ray = Ray::new([4., 0., 2.], [0., 0., -1.]).unwrap();
    assert!(scene.raycast(old_ray).is_none());
    let hit = scene.raycast(new_ray).unwrap();
    assert_eq!(hit.node, Some(node));
    assert_eq!(hit.object_id, Some("mesh".into()));
    assert!((hit.position[2] - 0.25).abs() < 1e-6);
    assert!((hit.uv[0] - 0.3).abs() < 1e-6);
    assert!(hit.normal[0] > 0.);
    assert!(before.scene(Camera::default()).raycast(old_ray).is_some());
    assert!(before.scene(Camera::default()).raycast(new_ray).is_none());
    graph.set_mesh(node, original).unwrap();
    assert!(
        graph
            .evaluate()
            .unwrap()
            .scene(Camera::default())
            .raycast(old_ray)
            .is_some()
    );
    assert!(scene.raycast(new_ray).is_some());
}

#[test]
fn vertex_replacement_validates_all_attributes_and_requires_explicit_tangents() {
    let source = Mesh::plane();
    assert!(matches!(
        source.with_vertices(vec![], None),
        Err(MeshUpdateError::VertexCount {
            expected: 4,
            actual: 0
        })
    ));
    let mut vertices = source.vertices().to_vec();
    vertices[3].uv[1] = f32::NAN;
    assert!(matches!(
        source.with_vertices(vertices, None),
        Err(MeshUpdateError::Geometry(MeshError::NonFiniteVertex {
            vertex: 3,
            attribute: VertexAttribute::Uv,
            component: 1
        }))
    ));
    let without = source
        .with_vertices(source.vertices().to_vec(), None)
        .unwrap();
    assert!(without.tangents().is_none());
    assert!(source.tangents().is_some());
    assert!(matches!(
        source.with_vertices(source.vertices().to_vec(), Some(vec![[1., 0., 0., 1.]])),
        Err(MeshUpdateError::Tangents(gpui_3d::TangentError::Count {
            expected: 4,
            actual: 1
        }))
    ));
    let mut vertices = source.vertices().to_vec();
    vertices[0].normal = [1., 0., 0.];
    assert!(matches!(
        source.with_vertices(vertices, Some(source.tangents().unwrap().to_vec())),
        Err(MeshUpdateError::Tangents(
            gpui_3d::TangentError::InvalidBasis { vertex: 0 }
        ))
    ));
    assert_eq!(source.vertices()[0].normal, [0., 0., 1.]);
}

#[test]
fn tangent_inputs_preserve_geometry_and_reject_undefined_bases() {
    use gpui_3d::TangentError;
    let source = Mesh::new(vertices(), vec![0, 1, 2]);
    let mesh = source.with_tangents(vec![[2., 0., 1., -1.]; 3]).unwrap();
    assert!(source.tangents().is_none());
    assert_eq!(mesh.tangents().unwrap(), &[[1., 0., 0., -1.]; 3]);
    assert!(std::ptr::eq(source.vertices(), mesh.vertices()));
    assert!(std::ptr::eq(source.indices(), mesh.indices()));
    let ray = Ray::new([0., 0., 1.], [0., 0., -1.]).unwrap();
    let hit = |mesh| {
        Scene::new()
            .object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))))
            .raycast(ray)
            .unwrap()
    };
    let before = hit(source.clone());
    let after = hit(mesh);
    assert_eq!(before.position, after.position);
    assert_eq!(before.normal, after.normal);
    assert_eq!(before.triangle_index, after.triangle_index);
    assert_eq!(
        source.with_tangents(vec![]).unwrap_err(),
        TangentError::Count {
            expected: 3,
            actual: 0
        }
    );
    for invalid in [
        [0.; 4],
        [0., 0., 1., 1.],
        [1., 0., 0., 0.],
        [f32::NAN, 0., 0., 1.],
    ] {
        let mut tangents = vec![[1., 0., 0., 1.]; 3];
        tangents[1] = invalid;
        assert_eq!(
            source.with_tangents(tangents).unwrap_err(),
            TangentError::InvalidBasis { vertex: 1 }
        );
    }
    assert_eq!(
        source
            .with_tangents(vec![[1., 0., 0., 1.], [1., 0., 0., -1.], [1., 0., 0., 1.]])
            .unwrap_err(),
        TangentError::MixedHandedness { triangle: 0 }
    );
    let mut data = vertices();
    data[2].normal = [0.; 3];
    assert_eq!(
        Mesh::new(data, vec![0, 1, 2])
            .with_tangents(vec![[1., 0., 0., 1.]; 3])
            .unwrap_err(),
        TangentError::InvalidBasis { vertex: 2 }
    );
}

#[test]
fn morph_weights_mix_signed_deltas_without_history_and_preserve_snapshot_queries() {
    let source = Mesh::plane();
    let targets = MorphTargets::new(
        source.clone(),
        [
            MorphTarget {
                positions: Some(vec![[2., 0., 0.]; 4].into()),
                ..Default::default()
            },
            MorphTarget {
                positions: Some(vec![[0., 4., 2.]; 4].into()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let a = targets.evaluate(&[0.5, -0.25]).unwrap();
    a.prepare_spatial_index();
    let b = targets.evaluate(&[0., 2.]).unwrap();
    assert_eq!(a.bounds().min(), [0.5, -1.5, -0.5]);
    assert_eq!(b.bounds().min(), [-0.5, 7.5, 4.]);
    let again = targets.evaluate(&[0.5, -0.25]).unwrap();
    assert_eq!(again.bounds(), a.bounds());
    assert!(std::ptr::eq(source.indices(), a.indices()));
    let ray = Ray::new([1., -1., 3.], [0., 0., -1.]).unwrap();
    let make_scene =
        |mesh| Scene::new().object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))));
    let snapshot = make_scene(a);
    let hit = snapshot.raycast(ray).unwrap();
    assert_eq!(hit.position, [1., -1., -0.5]);
    assert!((hit.uv[0] - 0.5).abs() < 1e-6 && (hit.uv[1] - 0.5).abs() < 1e-6);
    assert!(make_scene(b).raycast(ray).is_none());
    assert!(make_scene(source.clone()).raycast(ray).is_none());
    let zero = targets.evaluate(&[0., -0.]).unwrap();
    assert!(std::ptr::eq(zero.vertices(), source.vertices()));
    assert!(std::ptr::eq(
        zero.tangents().unwrap(),
        source.tangents().unwrap()
    ));
    assert!(snapshot.raycast(ray).is_some());
    let empty = MorphTargets::new(source.clone(), [])
        .unwrap()
        .evaluate(&[])
        .unwrap();
    assert!(std::ptr::eq(empty.vertices(), source.vertices()));
}

#[test]
fn morph_directions_blend_before_normalization_and_keep_tangent_handedness() {
    let source = Mesh::plane();
    let targets = MorphTargets::new(
        source.clone(),
        [
            MorphTarget {
                normals: Some(vec![[0., 1., 0.]; 4].into()),
                ..Default::default()
            },
            MorphTarget {
                tangents: Some(vec![[-1., 1., 0.]; 4].into()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let morphed = targets.evaluate(&[1., 1.]).unwrap();
    let k = std::f32::consts::FRAC_1_SQRT_2;
    for (index, vertex) in morphed.vertices().iter().enumerate() {
        assert_eq!(vertex.position, source.vertices()[index].position);
        assert_eq!(vertex.uv, source.vertices()[index].uv);
        let tangent = morphed.tangents().unwrap()[index];
        for (a, b) in vertex.normal.into_iter().zip([0., k, k]) {
            assert!((a - b).abs() < 1e-6);
        }
        for (a, b) in tangent.into_iter().zip([0., k, -k, -1.]) {
            assert!((a - b).abs() < 1e-6);
        }
    }
    let enormous = MorphTargets::new(
        source,
        [MorphTarget {
            normals: Some(vec![[0., f32::MAX, 0.]; 4].into()),
            ..Default::default()
        }],
    )
    .unwrap()
    .evaluate(&[f32::MAX])
    .unwrap();
    assert_eq!(enormous.vertices()[0].normal, [0., 1., 0.]);
}

#[test]
fn morph_normal_only_targets_preserve_zero_normals_but_reject_undefined_tangent_frames() {
    let source = Mesh::plane();
    let target = MorphTarget {
        normals: Some(vec![[0., 0., -1.]; 4].into()),
        ..Default::default()
    };
    let without_tangents = source
        .with_vertices(source.vertices().to_vec(), None)
        .unwrap();
    let zero = MorphTargets::new(without_tangents, [target.clone()])
        .unwrap()
        .evaluate(&[1.])
        .unwrap();
    assert!(zero.vertices().iter().all(|v| v.normal == [0.; 3]));
    assert!(matches!(
        MorphTargets::new(source.clone(), [target])
            .unwrap()
            .evaluate(&[1.]),
        Err(MorphError::Mesh(MeshUpdateError::Tangents(
            gpui_3d::TangentError::InvalidBasis { vertex: 0 }
        )))
    ));
    let cancelled_tangent = MorphTargets::new(
        source,
        [MorphTarget {
            tangents: Some(vec![[-1., 0., 0.]; 4].into()),
            ..Default::default()
        }],
    )
    .unwrap();
    assert!(matches!(
        cancelled_tangent.evaluate(&[1.]),
        Err(MorphError::Mesh(MeshUpdateError::Tangents(
            gpui_3d::TangentError::InvalidBasis { vertex: 0 }
        )))
    ));
}

#[test]
fn morph_validation_identifies_target_attributes_and_weight_failures() {
    let source = Mesh::plane();
    assert!(matches!(
        MorphTargets::new(source.clone(), [MorphTarget::default()]),
        Err(MorphError::EmptyTarget { target: 0 })
    ));
    for attribute in [
        MorphAttribute::Position,
        MorphAttribute::Normal,
        MorphAttribute::Tangent,
    ] {
        let target = |values| match attribute {
            MorphAttribute::Position => MorphTarget {
                positions: Some(values),
                ..Default::default()
            },
            MorphAttribute::Normal => MorphTarget {
                normals: Some(values),
                ..Default::default()
            },
            MorphAttribute::Tangent => MorphTarget {
                tangents: Some(values),
                ..Default::default()
            },
        };
        assert!(
            matches!(MorphTargets::new(source.clone(), [target(vec![[0.; 3]; 3].into())]), Err(MorphError::AttributeCount { target: 0, attribute: a, expected: 4, actual: 3 }) if a == attribute)
        );
        let mut invalid = vec![[0.; 3]; 4];
        invalid[3][2] = f32::NAN;
        assert!(
            matches!(MorphTargets::new(source.clone(), [target(invalid.into())]), Err(MorphError::NonFiniteDelta { target: 0, attribute: a, vertex: 3, component: 2 }) if a == attribute)
        );
    }
    let without_tangents = source
        .with_vertices(source.vertices().to_vec(), None)
        .unwrap();
    assert!(matches!(
        MorphTargets::new(
            without_tangents,
            [MorphTarget {
                tangents: Some(vec![[0.; 3]; 4].into()),
                ..Default::default()
            }]
        ),
        Err(MorphError::MissingBaseTangents { target: 0 })
    ));
    let targets = MorphTargets::new(
        source.clone(),
        [MorphTarget {
            positions: Some(vec![[f32::MAX, 0., 0.]; 4].into()),
            ..Default::default()
        }],
    )
    .unwrap();
    for weights in [&[][..], &[0., 1.][..]] {
        assert!(matches!(
            targets.evaluate(weights),
            Err(MorphError::WeightCount { expected: 1, .. })
        ));
    }
    for weight in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(matches!(
            targets.evaluate(&[weight]),
            Err(MorphError::NonFiniteWeight { target: 0 })
        ));
    }
    assert!(matches!(
        targets.evaluate(&[2.]),
        Err(MorphError::UnrepresentableResult {
            vertex: 0,
            attribute: MorphAttribute::Position
        })
    ));
    assert_eq!(source.bounds().min(), [-0.5, -0.5, 0.]);
    assert!(std::ptr::eq(
        targets.evaluate(&[0.]).unwrap().vertices(),
        source.vertices()
    ));
}
