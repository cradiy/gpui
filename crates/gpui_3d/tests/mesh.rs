use gpui_3d::{
    AffineTransform, Camera, Material, Mesh, MeshError, MeshUpdateError, MorphAttribute,
    MorphError, MorphTarget, MorphTargets, Node, Object, Ray, Scene, SceneError, SceneGraph, Skin,
    SkinError, SkinInfluence, Vertex, VertexAttribute,
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

#[test]
fn mesh_identity_distinguishes_attribute_updates_from_clones() {
    let mesh = Mesh::new(vertices(), vec![0, 1, 2]);
    assert!(mesh.ptr_eq(&mesh.clone()));
    assert!(!mesh.ptr_eq(&Mesh::new(vertices(), vec![0, 1, 2])));

    let uv = mesh.with_uv_set(7, vec![[0.5, 0.25]; 3]).unwrap();
    let colors = mesh
        .with_vertex_colors(vec![[0.5, 0.7, 0.9, 1.]; 3])
        .unwrap();
    for updated in [uv, colors] {
        assert_eq!(mesh.vertices().as_ptr(), updated.vertices().as_ptr());
        assert!(!mesh.ptr_eq(&updated));
        assert!(updated.ptr_eq(&updated.clone()));
    }
}

#[test]
fn corner_expansion_preserves_attributes_and_triangle_correspondence() {
    let positions = [
        [0., 0., 0.],
        [1., 0., 0.],
        [0., 1., 0.],
        [1., 1., 0.],
        [20.; 3],
    ];
    let source = Mesh::new(
        positions
            .into_iter()
            .enumerate()
            .map(|(i, position)| Vertex {
                position,
                normal: [0.3 + i as f32, 0.7, 1.1],
                uv: [-0., i as f32 * 0.3],
            })
            .collect(),
        vec![2, 0, 1, 2, 1, 3, 3, 3, 1],
    )
    .with_uv_set(7, (0..5).map(|i| [i as f32, -0.]).collect())
    .unwrap()
    .with_vertex_colors((0..5).map(|i| [i as f32 / 4., 0.5, 0.7, 0.8]).collect())
    .unwrap()
    .with_tangents_for_uv_set(7, vec![[0.17, 0.73, 0.39, -1.]; 5])
    .unwrap();
    let expanded = source.expand_corners(9).unwrap();
    assert_eq!(expanded.source_vertices(), source.indices());
    let mesh = expanded.mesh();
    assert_eq!(mesh.indices(), [0, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(mesh.triangle_count(), source.triangle_count());
    assert_eq!(mesh.uv_sets().collect::<Vec<_>>(), [0, 7]);
    assert_eq!(mesh.tangent_uv_set(), Some(7));
    for (out, &input) in expanded.source_vertices().iter().enumerate() {
        let input = input as usize;
        let a = mesh.vertices()[out];
        let b = source.vertices()[input];
        assert_eq!(a.position.map(f32::to_bits), b.position.map(f32::to_bits));
        assert_eq!(a.normal.map(f32::to_bits), b.normal.map(f32::to_bits));
        for set in [0, 7] {
            assert_eq!(
                mesh.uv_at(set, out).unwrap().map(f32::to_bits),
                source.uv_at(set, input).unwrap().map(f32::to_bits)
            );
        }
        assert_eq!(
            mesh.tangents().unwrap()[out].map(f32::to_bits),
            source.tangents().unwrap()[input].map(f32::to_bits)
        );
        assert_eq!(
            mesh.vertex_colors().unwrap()[out],
            source.vertex_colors().unwrap()[input]
        );
    }
    assert_eq!(source.vertex_count(), 5);
    assert_eq!(source.bounds().max(), [20.; 3]);
    assert_eq!(mesh.bounds().max(), [1., 1., 0.]);
    let repeated = mesh.expand_corners(9).unwrap();
    assert_eq!(repeated.source_vertices(), mesh.indices());
    assert_eq!(repeated.mesh().tangents(), mesh.tangents());
}

#[test]
fn corner_expansion_admission_and_absent_attributes() {
    let mesh = Mesh::new(vertices(), vec![2, 0, 1, 2, 2, 0]);
    for limit in [0, 3, 5] {
        assert_eq!(
            mesh.expand_corners(limit).unwrap_err(),
            gpui_3d::MeshExpansionError::VertexLimit { required: 6, limit }
        );
    }
    let (expanded, map) = mesh.expand_corners(6).unwrap().into_parts();
    assert_eq!(map, mesh.indices());
    assert_eq!(expanded.vertex_count(), 6);
    assert!(expanded.vertex_colors().is_none());
    assert!(expanded.tangents().is_none());
    assert!(expanded.tangent_uv_set().is_none());
    assert_eq!(expanded.uv_sets().collect::<Vec<_>>(), [0]);
}

#[test]
fn vertex_remapping_preserves_morph_skin_composition_and_weight_precision() {
    let base = Mesh::plane();
    let morph = MorphTargets::new(
        base.clone(),
        [
            MorphTarget {
                positions: Some((0..4).map(|i| [0.1 * i as f32, -0., 0.2]).collect()),
                normals: Some(vec![[0.1, 0.2, 0.]; 4].into()),
                tangents: Some(vec![[0., 0.1, 0.]; 4].into()),
            },
            MorphTarget {
                positions: Some(vec![[0., 0., -0.3]; 4].into()),
                ..Default::default()
            },
        ],
    )
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY; 2],
        (0..4).map(|i| {
            [
                SkinInfluence {
                    joint: 1,
                    weight: f32::MIN_POSITIVE,
                },
                SkinInfluence {
                    joint: 0,
                    weight: f32::MAX,
                },
                SkinInfluence {
                    joint: 0,
                    weight: f32::MAX / (i + 1) as f32,
                },
            ]
        }),
    )
    .unwrap();
    let expanded = base.expand_corners(6).unwrap();
    let map = expanded.source_vertices();
    let remapped_morph = morph.remap_vertices(expanded.mesh().clone(), map).unwrap();
    let remapped_skin = skin.remap_vertices(map).unwrap();
    assert!(std::ptr::eq(
        skin.inverse_bind_matrices(),
        remapped_skin.inverse_bind_matrices()
    ));
    for (output, &source) in map.iter().enumerate() {
        let before = skin.vertex_influences(source as usize).unwrap();
        let after = remapped_skin.vertex_influences(output).unwrap();
        assert_eq!(after, before);
        assert!(after[0].weight > 0.);
        assert_eq!(after[0].weight as f32, 0.);
        assert_eq!(
            remapped_morph.targets()[0].positions.as_ref().unwrap()[output].map(f32::to_bits),
            morph.targets()[0].positions.as_ref().unwrap()[source as usize].map(f32::to_bits)
        );
    }
    assert!(remapped_morph.targets()[1].normals.is_none());
    assert!(remapped_morph.targets()[1].tangents.is_none());
    let joints = [
        AffineTransform::from_translation([0.4, -0.3, 0.7]).unwrap(),
        AffineTransform::from_translation([-1., 0.5, 2.]).unwrap(),
    ];
    for weights in [[-0.25, 0.7], [0., 0.], [0.5, -1.]] {
        let original = skin
            .evaluate(&morph.evaluate(&weights).unwrap(), &joints)
            .unwrap();
        let result = remapped_skin
            .evaluate(&remapped_morph.evaluate(&weights).unwrap(), &joints)
            .unwrap();
        for (output, &source) in map.iter().enumerate() {
            assert_eq!(
                result.vertices()[output].position,
                original.vertices()[source as usize].position
            );
            assert_eq!(
                result.vertices()[output].normal,
                original.vertices()[source as usize].normal
            );
            assert_eq!(
                result.tangents().unwrap()[output],
                original.tangents().unwrap()[source as usize]
            );
        }
    }
    let identity = [0, 1, 2, 3];
    let rebound = morph.remap_vertices(base, &identity).unwrap();
    assert!(std::ptr::eq(rebound.targets(), morph.targets()));
    let rebound = skin.remap_vertices(&identity).unwrap();
    assert!(std::ptr::eq(
        rebound.vertex_influences(0).unwrap(),
        skin.vertex_influences(0).unwrap()
    ));
}

#[test]
fn vertex_remapping_validates_indices_counts_and_tangent_requirements() {
    let base = Mesh::plane();
    let morph = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            tangents: Some(vec![[0.; 3]; 4].into()),
            ..Default::default()
        }],
    )
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        vec![
            vec![SkinInfluence {
                joint: 0,
                weight: 1.
            }];
            4
        ],
    )
    .unwrap();
    assert_eq!(
        skin.remap_vertices(&[]).unwrap_err(),
        SkinError::EmptyVertices
    );
    assert_eq!(
        morph.remap_vertices(base.clone(), &[0, 1, 2]).unwrap_err(),
        MorphError::VertexCount {
            expected: 4,
            actual: 3
        }
    );
    for index in [4, u32::MAX] {
        let map = [0, 1, 2, index];
        assert_eq!(
            skin.remap_vertices(&map).unwrap_err(),
            SkinError::VertexIndex {
                vertex: index as usize,
                vertex_count: 4
            }
        );
        assert_eq!(
            morph.remap_vertices(base.clone(), &map).unwrap_err(),
            MorphError::VertexIndex {
                vertex: index as usize,
                vertex_count: 4
            }
        );
    }
    let without_tangents = Mesh::new(base.vertices().to_vec(), base.indices().to_vec());
    assert_eq!(
        morph
            .remap_vertices(without_tangents, &[0, 1, 2, 3])
            .unwrap_err(),
        MorphError::MissingBaseTangents { target: 0 }
    );
    let subset = Mesh::new(
        vec![base.vertices()[2], base.vertices()[0], base.vertices()[1]],
        vec![0, 1, 2],
    )
    .with_tangents(vec![[1., 0., 0., 1.]; 3])
    .unwrap();
    assert_eq!(
        morph
            .remap_vertices(subset, &[2, 0, 1])
            .unwrap()
            .base_mesh()
            .vertex_count(),
        3
    );
    assert_eq!(skin.remap_vertices(&[2, 0, 1]).unwrap().vertex_count(), 3);
}

#[test]
fn vertex_colors_validate_and_preserve_immutable_snapshots() {
    use gpui_3d::VertexColorError;
    let plane = Mesh::plane();
    let original = Mesh::new(plane.vertices().to_vec(), vec![0, 1, 2]);
    let colors = vec![
        [0.1, 0.2, 0.3, 0.4],
        [1., 0., 0., 1.],
        [0., 1., 0., 0.],
        [0., 0., 1., 0.5],
    ];
    let colored = original.with_vertex_colors(colors.clone()).unwrap();
    assert!(original.vertex_colors().is_none());
    assert!(std::ptr::eq(original.vertices(), colored.vertices()));
    assert!(std::ptr::eq(original.indices(), colored.indices()));
    assert_eq!(colored.vertex_colors().unwrap(), colors);
    let updated = colored
        .with_vertices(colored.vertices().to_vec(), None)
        .unwrap()
        .with_uv_set(7, vec![[0.; 2]; 4])
        .unwrap()
        .with_tangents_for_uv_set(7, plane.tangents().unwrap().to_vec())
        .unwrap();
    assert!(std::ptr::eq(
        updated.vertex_colors().unwrap(),
        colored.vertex_colors().unwrap()
    ));
    let replaced = updated.with_vertex_colors(vec![[1.; 4]; 4]).unwrap();
    assert_eq!(colored.vertex_colors().unwrap(), colors);
    assert_eq!(replaced.tangent_uv_set(), Some(7));
    assert_eq!(replaced.vertex_colors().unwrap(), [[1.; 4]; 4]);
    assert_eq!(
        colored.with_vertex_colors(vec![]).unwrap_err(),
        VertexColorError::Count {
            expected: 4,
            actual: 0
        }
    );
    for component in 0..4 {
        for value in [-0.1, 1.1, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = colors.clone();
            invalid[3][component] = value;
            assert_eq!(
                colored.with_vertex_colors(invalid).unwrap_err(),
                VertexColorError::InvalidComponent {
                    vertex: 3,
                    component
                }
            );
        }
    }
}

#[test]
fn vertex_colors_follow_split_morph_and_skin_correspondence() {
    let positions = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let uvs = [[0., 0.], [1., 0.], [0., 1.], [0., 1.]];
    let colors: Vec<_> = (0..4)
        .map(|i| [i as f32 / 4., 0.2, 0.4, 1. - i as f32 / 4.])
        .collect();
    let source = Mesh::new(
        (0..4)
            .map(|i| Vertex {
                position: positions[i],
                normal: [0.; 3],
                uv: uvs[i],
            })
            .collect(),
        vec![0, 1, 2, 0, 3, 1],
    )
    .with_vertex_colors(colors.clone())
    .unwrap();
    let normals = source.generate_normals(gpui_3d::NormalMode::Flat).unwrap();
    assert!(normals.mesh().vertex_count() > source.vertex_count());
    for (index, &original) in normals.source_vertices().iter().enumerate() {
        assert_eq!(
            normals.mesh().vertex_colors().unwrap()[index],
            colors[original as usize]
        );
    }
    let tangents = normals.mesh().generate_tangents().unwrap();
    for (index, &original) in tangents.source_vertices().iter().enumerate() {
        assert_eq!(
            tangents.mesh().vertex_colors().unwrap()[index],
            normals.mesh().vertex_colors().unwrap()[original as usize]
        );
    }
    let mesh = tangents.mesh();
    let morph = MorphTargets::new(
        mesh.clone(),
        [MorphTarget {
            positions: Some(vec![[0., 0., 1.]; mesh.vertex_count()].into()),
            ..Default::default()
        }],
    )
    .unwrap()
    .evaluate(&[0.5])
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        vec![
            [SkinInfluence {
                joint: 0,
                weight: 1.
            }];
            mesh.vertex_count()
        ],
    )
    .unwrap();
    let skinned = skin
        .evaluate(
            &morph,
            &[AffineTransform::from_translation([1., 0., 0.]).unwrap()],
        )
        .unwrap();
    assert!(std::ptr::eq(
        mesh.vertex_colors().unwrap(),
        skinned.vertex_colors().unwrap()
    ));
    assert_ne!(skinned.vertices()[0].position, mesh.vertices()[0].position);
}

#[test]
fn uv_snapshots_validate_sparse_sets_and_invalidate_replaced_basis() {
    use gpui_3d::UvSetError;
    let plane = Mesh::plane();
    let original = Mesh::new(plane.vertices().to_vec(), vec![0, 1, 2])
        .with_tangents(plane.tangents().unwrap().to_vec())
        .unwrap();
    let coordinates = vec![[-2., 3.], [4., 5.], [6., 7.], [8., 9.]];
    let attached = original.with_uv_set(u32::MAX, coordinates.clone()).unwrap();
    assert_eq!(attached.uv_sets().collect::<Vec<_>>(), [0, u32::MAX]);
    assert_eq!(original.uv_at(u32::MAX, 0), None);
    assert_eq!(attached.uv_at(1, 0), None);
    assert_eq!(attached.uv_at(u32::MAX, 4), None);
    assert!(std::ptr::eq(original.vertices(), attached.vertices()));
    assert!(std::ptr::eq(original.indices(), attached.indices()));
    assert!(std::ptr::eq(
        original.tangents().unwrap(),
        attached.tangents().unwrap()
    ));
    let replaced = attached.with_uv_set(0, coordinates.clone()).unwrap();
    assert!(replaced.tangents().is_none());
    for (i, uv) in coordinates.iter().enumerate() {
        assert_eq!(replaced.uv_at(0, i), Some(*uv));
        assert_eq!(replaced.uv_at(u32::MAX, i), Some(*uv));
        assert_eq!(attached.uv_at(0, i), Some(original.vertices()[i].uv));
    }
    for set in [0, u32::MAX] {
        assert_eq!(
            attached.with_uv_set(set, vec![]).unwrap_err(),
            UvSetError::Count {
                set,
                expected: 4,
                actual: 0
            }
        );
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut invalid = coordinates.clone();
            invalid[3][1] = value;
            assert_eq!(
                attached.with_uv_set(set, invalid).unwrap_err(),
                UvSetError::NonFinite {
                    set,
                    vertex: 3,
                    component: 1
                }
            );
        }
    }
    let updated = attached
        .with_vertices(attached.vertices().to_vec(), None)
        .unwrap();
    assert_eq!(updated.uv_at(u32::MAX, 3), Some(coordinates[3]));
    assert_eq!(attached.uv_at(u32::MAX, 3), Some(coordinates[3]));
    let overwritten = attached.with_uv_set(u32::MAX, vec![[0., 1.]; 4]).unwrap();
    assert_eq!(overwritten.uv_at(u32::MAX, 3), Some([0., 1.]));
    assert_eq!(attached.uv_at(u32::MAX, 3), Some(coordinates[3]));
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

fn near3(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-5, "{actual:?} != {expected:?}");
    }
}

#[test]
fn skin_bind_pose_and_world_pose_cancel_output_placement_without_accumulation() {
    let mesh = Mesh::plane();
    let bind = AffineTransform::from_translation([0., 2., 0.]).unwrap();
    let skin = Skin::new(
        [bind.inverse()],
        vec![
            [SkinInfluence {
                joint: 0,
                weight: 2.
            }];
            4
        ],
    )
    .unwrap();
    let rest = skin.evaluate(&mesh, &[bind]).unwrap();
    for (actual, base) in rest.vertices().iter().zip(mesh.vertices()) {
        near3(actual.position, base.position);
    }
    let mesh_world =
        AffineTransform::from_trs([3., -1., 2.], [0., 0., 1., 1.], [2., 3., 1.]).unwrap();
    let offset = AffineTransform::from_translation([1., 0., 0.5]).unwrap();
    let pose = mesh_world.compose(offset).unwrap().compose(bind).unwrap();
    let moved = skin.evaluate_world(&mesh, mesh_world, &[pose]).unwrap();
    let again = skin.evaluate_world(&mesh, mesh_world, &[pose]).unwrap();
    for ((actual, base), repeat) in moved
        .vertices()
        .iter()
        .zip(mesh.vertices())
        .zip(again.vertices())
    {
        near3(
            actual.position,
            [base.position[0] + 1., base.position[1], 0.5],
        );
        near3(actual.position, repeat.position);
        assert_eq!(actual.uv, base.uv);
    }
    near3(rest.bounds().min(), [-0.5, -0.5, 0.]);
    assert!(std::ptr::eq(moved.indices(), mesh.indices()));
    near3(mesh.vertices()[0].position, [-0.5, -0.5, 0.]);
}

#[test]
fn skin_mixes_all_influences_and_normalizes_extreme_weights_per_vertex() {
    let mesh = Mesh::plane();
    let joints: Vec<_> = (0..6)
        .map(|i| AffineTransform::from_translation([i as f32, 0., 0.]).unwrap())
        .collect();
    let influences = (0..4).map(|vertex| {
        (0..6).map(move |joint| SkinInfluence {
            joint,
            weight: if vertex == 0 { f32::MAX } else { 2. },
        })
    });
    let skin = Skin::new([AffineTransform::IDENTITY; 6], influences).unwrap();
    let moved = skin.evaluate(&mesh, &joints).unwrap();
    for (actual, base) in moved.vertices().iter().zip(mesh.vertices()) {
        near3(
            actual.position,
            [base.position[0] + 2.5, base.position[1], 0.],
        );
    }
    let tiny = Skin::new(
        [AffineTransform::IDENTITY],
        vec![
            [SkinInfluence {
                joint: 0,
                weight: f32::from_bits(1)
            }];
            4
        ],
    )
    .unwrap();
    near3(
        tiny.evaluate(&mesh, &joints[..1]).unwrap().bounds().min(),
        mesh.bounds().min(),
    );
}

#[test]
fn skin_blended_linear_transform_controls_normals_and_reflected_tangent_frames() {
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let mesh = Mesh::new(
        vertices()
            .into_iter()
            .map(|v| Vertex {
                normal: [k, k, 0.],
                ..v
            })
            .collect(),
        vec![0, 1, 2],
    )
    .with_tangents(vec![[k, -k, 0., -1.]; 3])
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY; 2],
        vec![
            [
                SkinInfluence {
                    joint: 0,
                    weight: 1.
                },
                SkinInfluence {
                    joint: 1,
                    weight: 3.
                }
            ];
            3
        ],
    )
    .unwrap();
    let scale = |s| AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], s).unwrap();
    let moved = skin
        .evaluate(&mesh, &[scale([2., 1., 1.]), scale([1., 3., 1.])])
        .unwrap();
    let k = 1. / 5_f32.sqrt();
    near3(moved.vertices()[0].normal, [2. * k, k, 0.]);
    let [x, y, z, w] = moved.tangents().unwrap()[0];
    near3([x, y, z], [k, -2. * k, 0.]);
    assert_eq!(w, -1.);
    let reflected = skin.evaluate(&mesh, &[scale([-1., 1., 1.]); 2]).unwrap();
    let k = std::f32::consts::FRAC_1_SQRT_2;
    near3(reflected.vertices()[0].normal, [-k, k, 0.]);
    let [x, y, z, w] = reflected.tangents().unwrap()[0];
    near3([x, y, z], [-k, -k, 0.]);
    assert_eq!(w, 1.);
    let zero = mesh
        .with_vertices(
            mesh.vertices()
                .iter()
                .map(|v| Vertex {
                    normal: [0.; 3],
                    ..*v
                })
                .collect(),
            None,
        )
        .unwrap();
    assert!(
        skin.evaluate(&zero, &[AffineTransform::IDENTITY; 2])
            .unwrap()
            .vertices()
            .iter()
            .all(|v| v.normal == [0.; 3])
    );
}

#[test]
fn skin_after_morph_updates_bounds_and_queries_without_changing_prior_snapshots() {
    let base = Mesh::plane();
    let morph = MorphTargets::new(
        base.clone(),
        [MorphTarget {
            positions: Some(vec![[1., 0., 0.]; 4].into()),
            ..Default::default()
        }],
    )
    .unwrap()
    .evaluate(&[1.])
    .unwrap();
    let skin = Skin::new(
        [AffineTransform::IDENTITY],
        vec![
            [SkinInfluence {
                joint: 0,
                weight: 1.
            }];
            4
        ],
    )
    .unwrap();
    let pose = AffineTransform::from_trs([0., 0., 1.], [0., 0., 1., 1.], [1.; 3]).unwrap();
    let result = skin.evaluate(&morph, &[pose]).unwrap();
    near3(result.bounds().min(), [-0.5, 0.5, 1.]);
    result.prepare_spatial_index();
    let material = Material::color(gpui::rgb(0xffffff));
    let scene = Scene::new().object(Object::new(result, material.clone()));
    let ray = Ray::new([0., 1., 4.], [0., 0., -1.]).unwrap();
    let hit = scene.raycast(ray).unwrap();
    near3(hit.position, [0., 1., 1.]);
    near3(hit.normal, [0., 0., 1.]);
    assert!((hit.uv[0] - 0.5).abs() < 1e-5 && (hit.uv[1] - 0.5).abs() < 1e-5);
    let rest = skin.evaluate(&morph, &[AffineTransform::IDENTITY]).unwrap();
    assert!(
        Scene::new()
            .object(Object::new(rest, material))
            .raycast(ray)
            .is_none()
    );
    near3(scene.raycast(ray).unwrap().position, [0., 1., 1.]);
    near3(base.bounds().min(), [-0.5, -0.5, 0.]);
}

#[test]
fn skin_rejects_invalid_bindings_pose_sizes_and_collapsed_blends() {
    let identity = AffineTransform::IDENTITY;
    assert_eq!(
        Skin::new(
            [],
            [[SkinInfluence {
                joint: 0,
                weight: 1.
            }]]
        )
        .unwrap_err(),
        SkinError::EmptyJoints
    );
    assert_eq!(
        Skin::new([identity], std::iter::empty::<[SkinInfluence; 1]>()).unwrap_err(),
        SkinError::EmptyVertices
    );
    for values in [
        vec![],
        vec![SkinInfluence {
            joint: 0,
            weight: 0.,
        }],
    ] {
        assert_eq!(
            Skin::new([identity], [values]).unwrap_err(),
            SkinError::MissingInfluences { vertex: 0 }
        );
    }
    for weight in [-1., f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            Skin::new([identity], [[SkinInfluence { joint: 0, weight }]]).unwrap_err(),
            SkinError::InvalidWeight {
                vertex: 0,
                influence: 0
            }
        );
    }
    assert_eq!(
        Skin::new(
            [identity],
            [[SkinInfluence {
                joint: 1,
                weight: 0.
            }]]
        )
        .unwrap_err(),
        SkinError::JointIndex {
            vertex: 0,
            influence: 0,
            joint: 1
        }
    );
    let skin = Skin::new(
        [identity; 2],
        vec![
            [
                SkinInfluence {
                    joint: 0,
                    weight: 1.
                },
                SkinInfluence {
                    joint: 1,
                    weight: 1.
                }
            ];
            4
        ],
    )
    .unwrap();
    let mesh = Mesh::plane();
    assert_eq!(
        skin.evaluate(&mesh, &[identity]).unwrap_err(),
        SkinError::JointCount {
            expected: 2,
            actual: 1
        }
    );
    assert_eq!(
        skin.evaluate(&Mesh::cube(), &[identity; 2]).unwrap_err(),
        SkinError::VertexCount {
            expected: 4,
            actual: 24
        }
    );
    let reflection = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [-1., 1., 1.]).unwrap();
    assert_eq!(
        skin.evaluate(&mesh, &[identity, reflection]).unwrap_err(),
        SkinError::InvalidVertexTransform { vertex: 0 }
    );
    let large = AffineTransform::from_translation([f32::MAX, 0., 0.]).unwrap();
    let huge_skin = Skin::new(
        [large],
        vec![
            [SkinInfluence {
                joint: 0,
                weight: 1.
            }];
            4
        ],
    )
    .unwrap();
    assert_eq!(
        huge_skin.evaluate(&mesh, &[large]).unwrap_err(),
        SkinError::InvalidJointTransform { joint: 0 }
    );
    let far = mesh
        .with_vertices(
            mesh.vertices()
                .iter()
                .map(|v| Vertex {
                    position: [f32::MAX, 0., 0.],
                    ..*v
                })
                .collect(),
            None,
        )
        .unwrap();
    let scale = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [2.; 3]).unwrap();
    assert_eq!(
        skin.evaluate(&far, &[scale; 2]).unwrap_err(),
        SkinError::UnrepresentablePosition { vertex: 0 }
    );
    assert!(skin.evaluate(&mesh, &[identity; 2]).is_ok());
    let folded = Skin::new(
        [identity; 2],
        (0..4).map(|vertex| {
            [SkinInfluence {
                joint: usize::from(vertex > 0),
                weight: 1.,
            }]
        }),
    )
    .unwrap();
    assert_eq!(
        folded.evaluate(&mesh, &[identity, reflection]).unwrap_err(),
        SkinError::Mesh(MeshUpdateError::Tangents(
            gpui_3d::TangentError::MixedHandedness { triangle: 0 }
        ))
    );
}
