use gpui_3d::{
    AffineTransform, Material, MaterialTexture, Mesh, MorphTarget, MorphTargets, Object,
    PbrMaterial, Ray, ResolvedTexture, Scene, Skin, SkinInfluence, SphereOptions,
    TangentGenerationError, TangentGenerationMode, TangentRepairKind, TextureSlot, TextureState,
    Vertex,
};

#[path = "tangents/topology.rs"]
mod topology;

fn quad(mirrored: bool) -> Mesh {
    let uv = if mirrored { [1., 0.] } else { [0., 1.] };
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([1., 0., 0.], [1., 0.]),
            ([1., 1., 0.], [1., 1.]),
            ([0., 1., 0.], uv),
        ]
        .map(|(position, uv)| Vertex {
            position,
            normal: [0., 0., 3.],
            uv,
        })
        .to_vec(),
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn near(a: [f32; 4], b: [f32; 4]) {
    for (a, b) in a.into_iter().zip(b) {
        assert!((a - b).abs() < 2e-6, "{a} != {b}");
    }
}

#[test]
fn selected_uv_frames_preserve_primary_coordinates_and_mirrored_seams() {
    for mirrored in [false, true] {
        let reference = quad(mirrored);
        let uv: Vec<_> = reference
            .vertices()
            .iter()
            .map(|v| [v.uv[1], v.uv[0]])
            .collect();
        let reference = reference
            .with_uv_set(0, uv.clone())
            .unwrap()
            .generate_tangents()
            .unwrap();
        let source = quad(false)
            .with_uv_set(0, vec![[0.; 2]; 4])
            .unwrap()
            .with_uv_set(7, uv)
            .unwrap();
        for mode in [
            TangentGenerationMode::Strict,
            TangentGenerationMode::Inherit,
            TangentGenerationMode::Repair,
        ] {
            let generated = source.generate_tangents_for_uv_set(7, mode).unwrap();
            assert_eq!(generated.mesh().tangent_uv_set(), Some(7));
            assert_eq!(generated.source_vertices(), reference.source_vertices());
            assert_eq!(generated.mesh().indices(), reference.mesh().indices());
            for (vertex, &index) in generated.source_vertices().iter().enumerate() {
                assert_eq!(generated.mesh().uv_at(0, vertex), Some([0.; 2]));
                assert_eq!(
                    generated.mesh().uv_at(7, vertex),
                    source.uv_at(7, index as usize)
                );
                near(
                    generated.mesh().tangents().unwrap()[vertex],
                    reference.mesh().tangents().unwrap()[vertex],
                );
            }
            assert!(generated.repairs().is_empty());
        }
        assert!(matches!(
            source.generate_tangents(),
            Err(TangentGenerationError::DegenerateUv { .. })
        ));
        assert_eq!(
            source
                .generate_tangents_for_uv_set(6, TangentGenerationMode::Repair)
                .unwrap_err(),
            TangentGenerationError::MissingUvSet { set: 6 }
        );
    }
}

#[test]
fn selected_uv_repairs_use_the_selected_derivative_and_report_degeneracy() {
    let source = quad(false).with_uv_set(5, vec![[0.; 2]; 4]).unwrap();
    assert_eq!(
        source
            .generate_tangents_for_uv_set(5, TangentGenerationMode::Strict)
            .unwrap_err(),
        TangentGenerationError::DegenerateUv { triangle: 0 }
    );
    let selected = source
        .generate_tangents_for_uv_set(5, TangentGenerationMode::Repair)
        .unwrap();
    let reference = source
        .with_uv_set(0, vec![[0.; 2]; 4])
        .unwrap()
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    assert_eq!(selected.repairs(), reference.repairs());
    assert!(!selected.repairs().is_empty());
    assert_eq!(selected.mesh().tangents(), reference.mesh().tangents());
    assert_eq!(selected.mesh().tangent_uv_set(), Some(5));
}

#[test]
fn tangent_set_associations_survive_deformation_and_invalidate_only_matching_uv_edits() {
    let base = quad(false).with_uv_set(7, vec![[0., 1.]; 4]).unwrap();
    let base = base
        .with_tangents_for_uv_set(7, vec![[0., 1., 0., -1.]; 4])
        .unwrap();
    assert!(matches!(
        base.with_tangents_for_uv_set(8, vec![[1., 0., 0., 1.]; 4]),
        Err(gpui_3d::TangentError::MissingUvSet { set: 8 })
    ));
    assert_eq!(
        base.with_uv_set(0, vec![[2.; 2]; 4])
            .unwrap()
            .tangent_uv_set(),
        Some(7)
    );
    assert_eq!(
        base.with_uv_set(7, vec![[2.; 2]; 4])
            .unwrap()
            .tangent_uv_set(),
        None
    );
    assert_eq!(base.tangent_uv_set(), Some(7));
    let morph = MorphTargets::new(
        base.clone(),
        vec![MorphTarget {
            positions: Some(vec![[0., 0., 0.25]; 4].into()),
            ..Default::default()
        }],
    )
    .unwrap()
    .evaluate(&[1.])
    .unwrap();
    let skin = Skin::new(
        vec![AffineTransform::IDENTITY],
        vec![
            vec![SkinInfluence {
                joint: 0,
                weight: 1.
            }];
            4
        ],
    )
    .unwrap();
    let result = skin
        .evaluate(
            &morph,
            &[AffineTransform::from_translation([0., 0., 1.]).unwrap()],
        )
        .unwrap();
    assert_eq!(morph.tangent_uv_set(), Some(7));
    assert_eq!(result.tangent_uv_set(), Some(7));
    for vertex in 0..4 {
        assert_eq!(result.uv_at(7, vertex), base.uv_at(7, vertex));
        near(
            result.tangents().unwrap()[vertex],
            base.tangents().unwrap()[vertex],
        );
        assert!((result.vertices()[vertex].position[2] - 1.25).abs() < 1e-6);
    }
    assert_eq!(
        base.with_vertices(base.vertices().to_vec(), None)
            .unwrap()
            .tangent_uv_set(),
        None
    );
}

#[test]
fn shared_planar_tangents_match_analytic_frames_and_replace_existing_data() {
    for mesh in [quad(false), Mesh::plane(), Mesh::cube()] {
        let generated = mesh.generate_tangents().unwrap();
        assert_eq!(generated.mesh().vertex_count(), mesh.vertex_count());
        assert_eq!(generated.mesh().indices(), mesh.indices());
        let expected = mesh
            .tangents()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| vec![[1., 0., 0., 1.]; mesh.vertex_count()]);
        for (&source, &actual) in generated
            .source_vertices()
            .iter()
            .zip(generated.mesh().tangents().unwrap())
        {
            near(actual, expected[source as usize]);
        }
    }
    let source = quad(false)
        .with_tangents(vec![[0., 1., 0., -1.]; 4])
        .unwrap();
    let replaced = source.generate_tangents().unwrap();
    assert_eq!(source.tangents().unwrap(), &[[0., 1., 0., -1.]; 4]);
    assert_eq!(replaced.mesh().tangents().unwrap(), &[[1., 0., 0., 1.]; 4]);
}

#[test]
fn mirrored_uvs_split_shared_vertices_without_changing_triangles_or_queries() {
    let source = quad(true)
        .with_uv_set(2, (0..4).map(|i| [i as f32, 3.]).collect())
        .unwrap();
    let generated = source.generate_tangents().unwrap();
    let output = generated.mesh();
    for (vertex, &source_vertex) in generated.source_vertices().iter().enumerate() {
        assert_eq!(
            output.uv_at(2, vertex),
            source.uv_at(2, source_vertex as usize)
        );
    }
    assert_eq!(output.vertex_count(), 6);
    assert_eq!(output.triangle_count(), source.triangle_count());
    assert!(source.tangents().is_none());
    for (&a, &b) in source.indices().iter().zip(output.indices()) {
        assert_eq!(generated.source_vertices()[b as usize], a);
        let (a, b) = (source.vertices()[a as usize], output.vertices()[b as usize]);
        assert_eq!(a.position, b.position);
        assert_eq!(a.normal, b.normal);
        assert_eq!(a.uv, b.uv);
    }
    let tangents = output.tangents().unwrap();
    for (triangle, indices) in output.indices().chunks_exact(3).enumerate() {
        for &index in indices {
            near(
                tangents[index as usize],
                if triangle == 0 {
                    [1., 0., 0., 1.]
                } else {
                    [0., 1., 0., -1.]
                },
            );
        }
    }
    for (triangle, [x, y]) in [[0.75, 0.25], [0.25, 0.75]].into_iter().enumerate() {
        let ray = Ray::new([x, y, 1.], [0., 0., -1.]).unwrap();
        for mesh in [&source, output] {
            let hit = Scene::new()
                .object(Object::new(mesh.clone(), Material::color(gpui::white())))
                .raycast(ray)
                .unwrap();
            assert_eq!(hit.triangle_index, triangle);
            assert!((hit.distance - 1.).abs() < 1e-6);
        }
    }
}

#[test]
fn smooth_frames_are_order_independent_and_do_not_weld_source_attributes() {
    let sphere = Mesh::sphere(SphereOptions {
        segments: [12, 6],
        ..Default::default()
    })
    .unwrap();
    let generated = sphere.generate_tangents().unwrap();
    let indices = sphere
        .indices()
        .chunks_exact(3)
        .rev()
        .flat_map(|face| [face[1], face[2], face[0]])
        .collect::<Vec<_>>();
    let reordered = Mesh::new(sphere.vertices().to_vec(), indices)
        .generate_tangents()
        .unwrap();
    for (triangle, face) in generated.mesh().indices().chunks_exact(3).enumerate() {
        let other =
            &reordered.mesh().indices()[(sphere.triangle_count() - triangle - 1) * 3..][..3];
        for corner in 0..3 {
            let a = face[corner] as usize;
            let b = other[(corner + 2) % 3] as usize;
            assert_eq!(
                generated.source_vertices()[a],
                reordered.source_vertices()[b]
            );
            near(
                generated.mesh().tangents().unwrap()[a],
                reordered.mesh().tangents().unwrap()[b],
            );
        }
    }
    // Separate source vertices may carry different external skin or morph data.
    let plane = quad(false);
    let vertices = plane
        .indices()
        .iter()
        .map(|&i| plane.vertices()[i as usize])
        .collect();
    let independent = Mesh::new(vertices, (0..6).collect())
        .generate_tangents()
        .unwrap();
    assert_eq!(independent.mesh().vertex_count(), 6);
    for tangent in independent.mesh().tangents().unwrap() {
        near(*tangent, [1., 0., 0., 1.]);
    }
}

#[test]
fn shared_corners_receive_an_angle_weighted_frame() {
    let source = quad(false);
    let mut vertices = source.vertices().to_vec();
    vertices[3].uv = [0.2, 0.8];
    let generated = Mesh::new(vertices, source.indices().to_vec())
        .generate_tangents()
        .unwrap();
    assert_eq!(generated.mesh().vertex_count(), 4);
    let second_length = 0.8_f32.hypot(0.2);
    let x = 1. + 0.8 / second_length;
    let y = -0.2 / second_length;
    let length = x.hypot(y);
    for (index, &source) in generated.source_vertices().iter().enumerate() {
        if source == 0 || source == 2 {
            near(
                generated.mesh().tangents().unwrap()[index],
                [x / length, y / length, 0., 1.],
            );
        }
    }
}

#[test]
fn generated_frames_satisfy_normal_map_preparation_and_preserve_normal_magnitude() {
    let source = quad(false);
    let coordinates = source.vertices().iter().map(|v| v.uv).collect();
    let selected = source
        .with_uv_set(7, coordinates)
        .unwrap()
        .generate_tangents_for_uv_set(7, TangentGenerationMode::Strict)
        .unwrap();
    let material = Material::color(gpui::white())
        .pbr(PbrMaterial::default())
        .normal_texture(MaterialTexture::new("normal.png"));
    let scene = Scene::new().object(Object::new(selected.mesh().clone(), material));
    let error = scene
        .prepare(1., None, |_| {
            panic!("invalid basis must fail before resource resolution")
        })
        .unwrap_err();
    assert!(error.to_string().contains("tangents for UV set 0"));
    for scale in [1e-30, 1., 1e30] {
        let source = quad(false);
        let vertices = source
            .vertices()
            .iter()
            .map(|v| Vertex {
                normal: [0., 0., scale],
                ..*v
            })
            .collect();
        let source = Mesh::new(vertices, source.indices().to_vec());
        let generated = source.generate_tangents().unwrap();
        for (vertex, tangent) in generated
            .mesh()
            .vertices()
            .iter()
            .zip(generated.mesh().tangents().unwrap())
        {
            assert_eq!(vertex.normal, [0., 0., scale]);
            near(*tangent, [1., 0., 0., 1.]);
        }
    }
    for segments in [[3, 2], [12, 6], [96, 48]] {
        let generated = Mesh::sphere(SphereOptions {
            radius: 0.7,
            segments,
        })
        .unwrap()
        .generate_tangents()
        .unwrap();
        let scene = Scene::new().object(Object::new(
            generated.mesh().clone(),
            Material::color(gpui::white())
                .pbr(PbrMaterial::default())
                .normal_texture(MaterialTexture::new("normal.png")),
        ));
        let mut slots = Vec::new();
        let prepared = scene
            .prepare(1., None, |request| {
                slots.push(request.slot);
                Ok(if request.slot == TextureSlot::Normal {
                    TextureState::Pending
                } else {
                    TextureState::Ready(ResolvedTexture::None)
                })
            })
            .unwrap();
        assert_eq!(slots.len(), 2);
        assert!(slots.contains(&TextureSlot::BaseColor));
        assert!(slots.contains(&TextureSlot::Normal));
        assert_eq!(prepared.pending_textures().len(), 1);
        assert_eq!(prepared.pending_textures()[0].slot, TextureSlot::Normal);
    }
}

#[test]
fn vertex_mapping_preserves_morph_and_skin_deformation_across_splits() {
    let source = quad(true);
    let mut vertices = source.vertices().to_vec();
    vertices.push(Vertex {
        position: [100.; 3],
        normal: [0.; 3],
        uv: [0.; 2],
    });
    let source = Mesh::new(vertices, source.indices().to_vec());
    let generated = source.generate_tangents().unwrap();
    assert!(generated.source_vertices().iter().all(|&index| index < 4));
    assert_ne!(generated.mesh().bounds(), source.bounds());
    let deltas: Vec<_> = (0..source.vertex_count())
        .map(|i| [0., 0., i as f32 * 0.1])
        .collect();
    let morph = |mesh, deltas: Vec<[f32; 3]>| {
        MorphTargets::new(
            mesh,
            [MorphTarget {
                positions: Some(deltas.into()),
                ..Default::default()
            }],
        )
        .unwrap()
        .evaluate(&[0.5])
        .unwrap()
    };
    let source_morph = morph(source.clone(), deltas.clone());
    let result_morph = morph(
        generated.mesh().clone(),
        generated
            .source_vertices()
            .iter()
            .map(|&i| deltas[i as usize])
            .collect(),
    );
    let influences: Vec<_> = (0..source.vertex_count())
        .map(|i| {
            [SkinInfluence {
                joint: i % 2,
                weight: 1.,
            }]
        })
        .collect();
    let joints = [
        AffineTransform::IDENTITY,
        AffineTransform::from_translation([0., 0., 0.5]).unwrap(),
    ];
    let source_skin = Skin::new([AffineTransform::IDENTITY; 2], influences.clone())
        .unwrap()
        .evaluate(&source_morph, &joints)
        .unwrap();
    let result_skin = Skin::new(
        [AffineTransform::IDENTITY; 2],
        generated
            .source_vertices()
            .iter()
            .map(|&i| influences[i as usize]),
    )
    .unwrap()
    .evaluate(&result_morph, &joints)
    .unwrap();
    for (result, &source) in result_skin
        .vertices()
        .iter()
        .zip(generated.source_vertices())
    {
        assert_eq!(
            result.position,
            source_skin.vertices()[source as usize].position
        );
    }
    assert_eq!(result_skin.indices(), generated.mesh().indices());
    assert!(result_skin.tangents().is_some());
}

#[test]
fn degenerate_inputs_report_locations_without_mutating_the_source() {
    for (vertices, indices, expected) in [
        (
            quad(false).vertices().to_vec(),
            vec![0, 1, 2, 0, 0, 3],
            TangentGenerationError::DegenerateGeometry { triangle: 1 },
        ),
        (
            {
                let mut v = quad(false).vertices().to_vec();
                v[3].uv = v[0].uv;
                v
            },
            vec![0, 1, 2, 0, 2, 3],
            TangentGenerationError::DegenerateUv { triangle: 1 },
        ),
        (
            {
                let mut v = quad(false).vertices().to_vec();
                v[3].normal = [0.; 3];
                v
            },
            vec![0, 1, 2, 0, 2, 3],
            TangentGenerationError::InvalidNormal { vertex: 3 },
        ),
    ] {
        let mesh = Mesh::new(vertices, indices);
        assert_eq!(mesh.generate_tangents().unwrap_err(), expected);
        assert!(mesh.tangents().is_none());
    }
    for scale in [1e-30, 1e30] {
        let mesh = quad(false);
        for scale_uv in [false, true] {
            let vertices = mesh
                .vertices()
                .iter()
                .map(|v| {
                    if scale_uv {
                        Vertex {
                            uv: v.uv.map(|v| v * scale),
                            ..*v
                        }
                    } else {
                        Vertex {
                            position: v.position.map(|v| v * scale),
                            ..*v
                        }
                    }
                })
                .collect();
            let mesh = Mesh::new(vertices, mesh.indices().to_vec());
            for mode in [
                TangentGenerationMode::Strict,
                TangentGenerationMode::Inherit,
                TangentGenerationMode::Repair,
            ] {
                assert_eq!(
                    mesh.generate_tangents_with_mode(mode).unwrap_err(),
                    TangentGenerationError::Unrepresentable { triangle: 0 }
                );
            }
        }
    }
    let source = quad(false);
    let vertices = source
        .vertices()
        .iter()
        .map(|v| Vertex {
            normal: [1., 0., 0.],
            ..*v
        })
        .collect();
    let mesh = Mesh::new(vertices, source.indices().to_vec());
    assert_eq!(
        mesh.generate_tangents().unwrap_err(),
        TangentGenerationError::InvalidBasis {
            triangle: 0,
            corner: 0
        }
    );
}

#[test]
fn inherited_degenerate_faces_preserve_good_frames_and_source_correspondence() {
    let vertices: Vec<_> = [
        ([0., 0., 0.], [0., 0.]),
        ([1., 0., 0.], [1., 0.]),
        ([1., 1., 0.], [1., 1.]),
        ([0., 1., 0.], [2., 2.]),
        ([-1., 0., 0.], [-1., 0.]),
    ]
    .map(|(position, uv)| Vertex {
        position,
        uv,
        normal: [0., 0., 1.],
    })
    .to_vec();
    let good = Mesh::new(vertices.clone(), vec![0, 1, 2, 0, 3, 4]);
    let baseline = good.generate_tangents().unwrap();
    for faces in [
        vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 0, 2],
        vec![0, 0, 2, 0, 3, 4, 0, 2, 3, 0, 1, 2],
    ] {
        let mesh = Mesh::new(vertices.clone(), faces.clone());
        for mode in [
            TangentGenerationMode::Inherit,
            TangentGenerationMode::Repair,
        ] {
            let output = mesh.generate_tangents_with_mode(mode).unwrap();
            assert!(output.repairs().is_empty());
            assert_eq!(output.mesh().index_count(), faces.len());
            for (&output_index, &source) in output.mesh().indices().iter().zip(&faces) {
                assert_eq!(output.source_vertices()[output_index as usize], source);
                let actual = output.mesh().vertices()[output_index as usize];
                let expected = vertices[source as usize];
                assert_eq!(
                    (actual.position, actual.normal, actual.uv),
                    (expected.position, expected.normal, expected.uv)
                );
                let base_index = baseline
                    .source_vertices()
                    .iter()
                    .position(|&i| i == source)
                    .unwrap();
                near(
                    output.mesh().tangents().unwrap()[output_index as usize],
                    baseline.mesh().tangents().unwrap()[base_index],
                );
            }
        }
    }
}

#[test]
fn repairs_recover_skinny_triangle_derivatives_without_replacing_valid_frames() {
    let mesh = Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([1., 0., 0.], [1., 0.]),
            ([2., 1e-5, 0.], [2., 1.]),
        ]
        .map(|(position, uv)| Vertex {
            position,
            uv,
            normal: [0., 0., 1.],
        })
        .to_vec(),
        vec![0, 1, 2],
    );
    assert!(matches!(
        mesh.generate_tangents(),
        Err(TangentGenerationError::InvalidBasis { .. })
    ));
    let output = mesh
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    assert_eq!(output.repairs().len(), 2);
    assert!(
        output
            .repairs()
            .iter()
            .all(|repair| repair.triangle == 0
                && repair.kind == TangentRepairKind::TriangleDerivative)
    );
    for &tangent in output.mesh().tangents().unwrap() {
        near(tangent, [1., 0., 0., 1.]);
    }
    assert_eq!(output.source_vertices(), [0, 1, 2]);
    assert!(mesh.tangents().is_none());
}

#[test]
fn undefined_uv_islands_report_deterministic_orthonormal_repairs() {
    for normal in [[1., 2., 3.], [0., 0., -1.], [1e-30, 0., 0.], [0., 1e30, 0.]] {
        let mesh = quad(false);
        let vertices: Vec<_> = mesh
            .vertices()
            .iter()
            .map(|v| Vertex {
                normal,
                uv: [0.5; 2],
                ..*v
            })
            .collect();
        let mesh = Mesh::new(vertices, mesh.indices().to_vec());
        assert!(matches!(
            mesh.generate_tangents_with_mode(TangentGenerationMode::Inherit),
            Err(TangentGenerationError::InvalidBasis { .. })
        ));
        let output = mesh
            .generate_tangents_with_mode(TangentGenerationMode::Repair)
            .unwrap();
        assert_eq!(output.mesh().indices(), mesh.indices());
        for (actual, expected) in output.mesh().vertices().iter().zip(mesh.vertices()) {
            assert_eq!(
                (actual.position, actual.normal, actual.uv),
                (expected.position, expected.normal, expected.uv)
            );
        }
        assert_eq!(output.repairs().len(), mesh.index_count());
        let length = normal
            .iter()
            .map(|&v| f64::from(v).powi(2))
            .sum::<f64>()
            .sqrt();
        for (offset, repair) in output.repairs().iter().enumerate() {
            assert_eq!((repair.triangle, repair.corner), (offset / 3, offset % 3));
            assert_eq!(repair.kind, TangentRepairKind::OrthonormalBasis);
        }
        for tangent in output.mesh().tangents().unwrap() {
            let dot = normal
                .iter()
                .zip(tangent)
                .map(|(&n, &t)| f64::from(n) / length * f64::from(t))
                .sum::<f64>();
            assert!(dot.abs() < 1e-6);
            assert!((tangent[..3].iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1e-6);
            assert_eq!(tangent[3], 1.);
        }
        let repeat = mesh
            .generate_tangents_with_mode(TangentGenerationMode::Repair)
            .unwrap();
        assert_eq!(repeat.mesh().tangents(), output.mesh().tangents());
    }
}

#[test]
fn repaired_corners_keep_inherited_mirrored_handedness() {
    let mesh = quad(false);
    let vertices = mesh
        .vertices()
        .iter()
        .enumerate()
        .map(|(i, v)| Vertex {
            uv: if i == 3 { [0.; 2] } else { [-v.uv[0], v.uv[1]] },
            ..*v
        })
        .collect();
    let mesh = Mesh::new(vertices, mesh.indices().to_vec());
    let output = mesh
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    assert_eq!(output.repairs().len(), 1);
    assert_eq!(
        (output.repairs()[0].triangle, output.repairs()[0].corner),
        (1, 2)
    );
    assert_eq!(
        output.repairs()[0].kind,
        TangentRepairKind::OrthonormalBasis
    );
    assert!(
        output
            .mesh()
            .tangents()
            .unwrap()
            .iter()
            .all(|t| t[3] == -1.)
    );
}
