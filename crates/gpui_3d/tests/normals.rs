use gpui_3d::{
    AffineTransform, Material, Mesh, MorphTarget, MorphTargets, NormalGenerationError, NormalMode,
    Object, Ray, Scene, Skin, SkinInfluence, Vertex,
};

fn folded() -> Mesh {
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([2., 0., 0.], [1., 0.]),
            ([0., 2., 0.], [0., 1.]),
            ([0., 0., 1.], [0., 1.]),
            ([100., 100., 100.], [0., 0.]),
        ]
        .map(|(position, uv)| Vertex {
            position,
            uv,
            normal: [0.; 3],
        })
        .to_vec(),
        vec![0, 1, 2, 0, 3, 1],
    )
}

fn close(actual: [f32; 3], expected: [f32; 3]) {
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 2e-6, "{actual:?} != {expected:?}");
    }
}

#[test]
fn flat_normals_split_folds_and_preserve_corner_attributes_and_identity() {
    let source = folded();
    let generated = source.generate_normals(NormalMode::Flat).unwrap();
    assert_eq!(generated.source_vertices(), [0, 1, 2, 0, 3, 1]);
    let mesh = generated.mesh();
    assert_eq!(mesh.vertex_count(), 6);
    assert_eq!(mesh.indices(), [0, 1, 2, 3, 4, 5]);
    for (corner, &output) in mesh.indices().iter().enumerate() {
        let original = source.vertices()[source.indices()[corner] as usize];
        let actual = mesh.vertices()[output as usize];
        assert_eq!(actual.position, original.position);
        assert_eq!(actual.uv, original.uv);
        close(
            actual.normal,
            if corner < 3 {
                [0., 0., 1.]
            } else {
                [0., 1., 0.]
            },
        );
    }
    assert_eq!(mesh.bounds().max(), [2., 2., 1.]);
    assert_eq!(source.bounds().max(), [100.; 3]);
    assert!(source.vertices().iter().all(|v| v.normal == [0.; 3]));
    let scene = Scene::new().object(Object::new(mesh.clone(), Material::color(gpui::white())));
    for (ray, triangle, normal) in [
        (
            Ray::new([0.25, 0.25, 2.], [0., 0., -1.]).unwrap(),
            0,
            [0., 0., 1.],
        ),
        (
            Ray::new([0.25, 2., 0.25], [0., -1., 0.]).unwrap(),
            1,
            [0., 1., 0.],
        ),
    ] {
        let hit = scene.raycast(ray).unwrap();
        assert_eq!(hit.triangle_index, triangle);
        close(hit.normal, normal);
    }
}

#[test]
fn smooth_normals_use_face_area_without_welding_separate_source_indices() {
    let source = folded();
    let generated = source.generate_normals(NormalMode::Smooth).unwrap();
    assert_eq!(generated.source_vertices(), [0, 1, 2, 3]);
    assert_eq!(generated.mesh().indices(), source.indices());
    let expected = [0., 1. / 5_f32.sqrt(), 2. / 5_f32.sqrt()];
    for vertex in &generated.mesh().vertices()[..2] {
        close(vertex.normal, expected);
    }
    close(generated.mesh().vertices()[2].normal, [0., 0., 1.]);
    close(generated.mesh().vertices()[3].normal, [0., 1., 0.]);

    let mut vertices = source.vertices().to_vec();
    vertices.push(vertices[0]);
    vertices.push(vertices[1]);
    let split = Mesh::new(vertices, vec![0, 1, 2, 5, 3, 6]);
    let split = split.generate_normals(NormalMode::Smooth).unwrap();
    assert_eq!(split.mesh().vertex_count(), 6);
    close(split.mesh().vertices()[0].normal, [0., 0., 1.]);
    close(split.mesh().vertices()[3].normal, [0., 1., 0.]);
}

#[test]
fn planar_generation_reuses_vertices_clears_tangents_and_respects_winding() {
    let source = Mesh::plane();
    assert!(source.tangents().is_some());
    for mode in [NormalMode::Flat, NormalMode::Smooth] {
        let generated = source.generate_normals(mode).unwrap();
        assert_eq!(generated.mesh().vertex_count(), source.vertex_count());
        assert!(generated.mesh().tangents().is_none());
        assert!(source.tangents().is_some());
        let tangents = generated.mesh().generate_tangents().unwrap();
        for tangent in tangents.mesh().tangents().unwrap() {
            close([tangent[0], tangent[1], tangent[2]], [1., 0., 0.]);
        }
        let mut reversed = source.indices().to_vec();
        for triangle in reversed.chunks_exact_mut(3) {
            triangle.swap(1, 2);
        }
        let reversed = Mesh::new(source.vertices().to_vec(), reversed)
            .generate_normals(mode)
            .unwrap();
        for vertex in reversed.mesh().vertices() {
            close(vertex.normal, [0., 0., -1.]);
        }
    }
}

#[test]
fn degenerate_and_cancelling_faces_return_context_without_mutation() {
    let source = folded();
    for mode in [NormalMode::Flat, NormalMode::Smooth] {
        let invalid = Mesh::new(source.vertices().to_vec(), vec![0, 1, 2, 0, 1, 1]);
        assert_eq!(
            invalid.generate_normals(mode).unwrap_err(),
            NormalGenerationError::DegenerateGeometry { triangle: 1 }
        );
        assert_eq!(invalid.indices(), [0, 1, 2, 0, 1, 1]);
        assert!(
            invalid
                .vertices()
                .iter()
                .all(|vertex| vertex.normal == [0.; 3])
        );
    }
    let opposing = Mesh::new(source.vertices().to_vec(), vec![0, 1, 2, 0, 2, 1]);
    assert_eq!(
        opposing.generate_normals(NormalMode::Smooth).unwrap_err(),
        NormalGenerationError::InvalidNormal { vertex: 0 }
    );
    assert_eq!(
        opposing
            .generate_normals(NormalMode::Flat)
            .unwrap()
            .mesh()
            .vertex_count(),
        6
    );
}

#[test]
fn normal_generation_handles_extreme_finite_position_scales() {
    for extent in [f32::from_bits(1), 1e-20, 1., 1e20, f32::MAX] {
        let mesh = Mesh::new(
            [[-extent, 0., 0.], [extent, 0., 0.], [0., extent, 0.]]
                .map(|position| Vertex {
                    position,
                    normal: [0.; 3],
                    uv: [0.; 2],
                })
                .to_vec(),
            vec![0, 1, 2],
        );
        for mode in [NormalMode::Flat, NormalMode::Smooth] {
            let generated = mesh.generate_normals(mode).unwrap();
            for vertex in generated.mesh().vertices() {
                close(vertex.normal, [0., 0., 1.]);
            }
        }
    }
}

#[test]
fn normal_generation_preserves_near_collinear_face_winding() {
    let vertices = [
        [0.; 3],
        [16_777_216., 16_777_215., 0.],
        [16_777_215., 16_777_214., 0.],
    ]
    .map(|position| Vertex {
        position,
        normal: [0.; 3],
        uv: [0.; 2],
    });
    for indices in [[0, 1, 2], [1, 2, 0], [2, 0, 1]] {
        for reversed in [false, true] {
            let mut indices = indices.to_vec();
            if reversed {
                indices.swap(1, 2);
            }
            let mesh = Mesh::new(vertices.to_vec(), indices);
            for mode in [NormalMode::Flat, NormalMode::Smooth] {
                let generated = mesh.generate_normals(mode).unwrap();
                for vertex in generated.mesh().vertices() {
                    close(vertex.normal, [0., 0., if reversed { 1. } else { -1. }]);
                }
            }
        }
    }
}

#[test]
fn generated_vertex_maps_compose_with_tangents_morph_and_skin_inputs() {
    let source = folded()
        .with_uv_set(7, (0..5).map(|i| [i as f32, -2.]).collect())
        .unwrap();
    let (normal_mesh, normal_sources) = source
        .generate_normals(NormalMode::Flat)
        .unwrap()
        .into_parts();
    let (mesh, tangent_sources) = normal_mesh.generate_tangents().unwrap().into_parts();
    let sources: Vec<_> = tangent_sources
        .iter()
        .map(|&i| normal_sources[i as usize])
        .collect();
    let source_deltas: Vec<_> = (0..source.vertex_count())
        .map(|i| [0., 0., i as f32 * 0.1])
        .collect();
    let targets = MorphTargets::new(
        mesh,
        vec![MorphTarget {
            positions: Some(sources.iter().map(|&i| source_deltas[i as usize]).collect()),
            ..Default::default()
        }],
    )
    .unwrap();
    let morphed = targets.evaluate(&[1.]).unwrap();
    for (vertex, &index) in sources.iter().enumerate() {
        assert_eq!(morphed.uv_at(7, vertex), source.uv_at(7, index as usize));
    }
    let influences: Vec<_> = sources
        .iter()
        .map(|&i| {
            vec![SkinInfluence {
                joint: i as usize % 2,
                weight: 1.,
            }]
        })
        .collect();
    let skin = Skin::new(vec![AffineTransform::IDENTITY; 2], influences).unwrap();
    let deformed = skin
        .evaluate(
            &morphed,
            &[
                AffineTransform::from_translation([3., 0., 0.]).unwrap(),
                AffineTransform::from_translation([3., 0., 0.2]).unwrap(),
            ],
        )
        .unwrap();
    for (output, &index) in sources.iter().enumerate() {
        assert_eq!(deformed.uv_at(7, output), source.uv_at(7, index as usize));
    }
    for (vertex, &index) in deformed.vertices().iter().zip(&sources) {
        let source = source.vertices()[index as usize];
        close(
            vertex.position,
            [
                source.position[0] + 3.,
                source.position[1],
                source.position[2] + index as f32 * 0.1 + (index % 2) as f32 * 0.2,
            ],
        );
    }
    let scene = Scene::new().object(Object::new(deformed, Material::color(gpui::white())));
    let hit = scene
        .raycast(Ray::new([3.25, 0.25, 2.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert_eq!(hit.triangle_index, 0);
    close(hit.position, [3.25, 0.25, 0.0625]);
    close(hit.normal, [0., 0., 1.]);
}
