use gpui_3d::{AffineTransform, Mesh, NormalizedSkinInfluence, Skin, SkinError, SkinInfluence};

fn influence(joint: usize, weight: f32) -> SkinInfluence {
    SkinInfluence { joint, weight }
}

#[test]
fn palettes_retain_mesh_space_matrices_across_samples_clones_and_vertex_remaps() {
    let mesh = Mesh::plane();
    let world = AffineTransform::from_translation([5., 2., -3.]).unwrap();
    let binds = [
        AffineTransform::from_translation([-1., 0., 0.]).unwrap(),
        AffineTransform::from_translation([0., -2., 0.]).unwrap(),
    ];
    let deltas = [[0.5, 0., 0.], [0., 1., 0.]];
    let joints: Vec<_> = deltas
        .iter()
        .zip(binds)
        .map(|(&delta, bind)| {
            world
                .compose(AffineTransform::from_translation(delta).unwrap())
                .unwrap()
                .compose(bind.inverse())
                .unwrap()
        })
        .collect();
    let source = Skin::new(binds, (0..4).map(|v| [influence(v % 2, 1.)])).unwrap();
    let palette = source.palette(world, &joints).unwrap();
    let retained = palette.clone();
    assert!(std::ptr::eq(palette.matrices(), retained.matrices()));
    for (&matrix, delta) in palette.matrices().iter().zip(deltas) {
        assert_eq!(
            matrix,
            AffineTransform::from_translation(delta).unwrap().matrix()
        );
    }
    let cloned = source.clone();
    let expanded = mesh.expand_corners(6).unwrap();
    let remapped = source.remap_vertices(expanded.source_vertices()).unwrap();
    let unrelated = Skin::new(binds, (0..4).map(|v| [influence(v % 2, 1.)])).unwrap();
    assert!(matches!(
        unrelated.evaluate_with_palette(&mesh, &palette),
        Err(SkinError::PaletteBindingMismatch)
    ));
    assert!(matches!(
        source.evaluate_with_palette(&Mesh::cube(), &palette),
        Err(SkinError::VertexCount { .. })
    ));
    let newer = source.palette(world, &[world; 2]).unwrap();
    assert_ne!(palette.matrices(), newer.matrices());
    drop((source, palette));
    let sampled = cloned.evaluate_with_palette(&mesh, &retained).unwrap();
    let corners = remapped
        .evaluate_with_palette(expanded.mesh(), &retained)
        .unwrap();
    for (index, vertex) in sampled.vertices().iter().enumerate() {
        let expected = std::array::from_fn(|axis| {
            mesh.vertices()[index].position[axis] + deltas[index % 2][axis]
        });
        assert_eq!(vertex.position, expected);
        assert_eq!(vertex.normal, mesh.vertices()[index].normal);
    }
    for (vertex, &source) in corners.vertices().iter().zip(expanded.source_vertices()) {
        assert_eq!(
            vertex.position,
            sampled.vertices()[source as usize].position
        );
        assert_eq!(vertex.normal, sampled.vertices()[source as usize].normal);
    }
    assert_eq!(sampled.tangents(), mesh.tangents());
    assert_eq!(corners.tangents(), expanded.mesh().tangents());
}

#[test]
fn palette_admission_checks_every_joint_before_vertex_blending() {
    let source = Skin::new(
        [AffineTransform::IDENTITY; 2],
        (0..4).map(|_| [influence(0, 1.), influence(1, 1.)]),
    )
    .unwrap();
    assert!(matches!(
        source.palette(AffineTransform::IDENTITY, &[AffineTransform::IDENTITY]),
        Err(SkinError::JointCount {
            expected: 2,
            actual: 1
        })
    ));
    let large = AffineTransform::from_translation([f32::MAX, 0., 0.]).unwrap();
    assert!(matches!(
        source.palette(large.inverse(), &[large.inverse(), large]),
        Err(SkinError::InvalidJointTransform { joint: 1 })
    ));
    let reflection = AffineTransform::from_trs([0.; 3], [0., 0., 0., 1.], [-1., 1., 1.]).unwrap();
    let palette = source
        .palette(
            AffineTransform::IDENTITY,
            &[AffineTransform::IDENTITY, reflection],
        )
        .unwrap();
    assert!(matches!(
        source.evaluate_with_palette(&Mesh::plane(), &palette),
        Err(SkinError::InvalidVertexTransform { vertex: 0 })
    ));
}

#[test]
fn influence_views_preserve_normalized_order_duplicates_and_shared_storage() {
    let skin = Skin::new(
        [AffineTransform::IDENTITY; 2],
        [
            vec![
                influence(1, 2.),
                influence(0, 0.),
                influence(0, 1.),
                influence(1, 1.),
            ],
            vec![influence(0, 7.)],
        ],
    )
    .unwrap();
    let clone = skin.clone();
    let weights = skin.vertex_influences(0).unwrap();
    assert_eq!(
        weights,
        &[
            NormalizedSkinInfluence {
                joint: 1,
                weight: 0.5
            },
            NormalizedSkinInfluence {
                joint: 0,
                weight: 0.25
            },
            NormalizedSkinInfluence {
                joint: 1,
                weight: 0.25
            },
        ]
    );
    assert!(std::ptr::eq(weights, clone.vertex_influences(0).unwrap()));
    assert_eq!(
        skin.vertex_influences(1).unwrap(),
        &[NormalizedSkinInfluence {
            joint: 0,
            weight: 1.
        }]
    );
    for vertex in [2, usize::MAX] {
        assert_eq!(
            skin.vertex_influences(vertex),
            Err(SkinError::VertexIndex {
                vertex,
                vertex_count: 2
            })
        );
    }
    drop(skin);
    assert_eq!(clone.vertex_influences(0).unwrap().len(), 3);
}

#[test]
fn influence_views_keep_sub_f32_weights_and_avoid_large_sum_overflow() {
    let tiny = f32::from_bits(1);
    let skin = Skin::new(
        [AffineTransform::IDENTITY; 2],
        [[
            influence(0, tiny),
            influence(1, f32::MAX),
            influence(1, f32::MAX),
        ]],
    )
    .unwrap();
    let values = skin.vertex_influences(0).unwrap();
    assert!(
        values
            .iter()
            .all(|value| value.weight.is_finite() && value.weight > 0.)
    );
    assert_eq!(
        values[0].weight,
        f64::from(tiny) / (2. * f64::from(f32::MAX))
    );
    assert_eq!(values[0].weight as f32, 0.);
    assert!((values.iter().map(|value| value.weight).sum::<f64>() - 1.).abs() < 1e-15);
}

#[test]
fn inspected_weights_match_deformation_and_rebuilt_bindings_remain_independent() {
    let mesh = Mesh::plane();
    let skin = Skin::new(
        [AffineTransform::IDENTITY; 2],
        (0..mesh.vertex_count())
            .map(|vertex| [influence(0, 1.), influence(1, (vertex + 1) as f32)]),
    )
    .unwrap();
    let joints = [
        AffineTransform::IDENTITY,
        AffineTransform::from_translation([4., 0., 0.]).unwrap(),
    ];
    let original = skin.evaluate(&mesh, &joints).unwrap();
    for (vertex, (source, result)) in mesh.vertices().iter().zip(original.vertices()).enumerate() {
        let displacement: f64 = skin
            .vertex_influences(vertex)
            .unwrap()
            .iter()
            .map(|value| joints[value.joint].transform_point([0.; 3])[0] as f64 * value.weight)
            .sum();
        assert!(
            (f64::from(result.position[0]) - f64::from(source.position[0]) - displacement).abs()
                < 1e-6
        );
    }
    let mut edited: Vec<Vec<SkinInfluence>> = (0..skin.vertex_count())
        .map(|vertex| {
            skin.vertex_influences(vertex)
                .unwrap()
                .iter()
                .map(|value| influence(value.joint, value.weight as f32))
                .collect()
        })
        .collect();
    edited[0] = vec![influence(1, 1.)];
    let updated = Skin::new(skin.inverse_bind_matrices().iter().copied(), edited).unwrap();
    let changed = updated.evaluate(&mesh, &joints).unwrap();
    assert_eq!(
        changed.vertices()[0].position[0],
        mesh.vertices()[0].position[0] + 4.
    );
    assert_ne!(
        changed.vertices()[0].position,
        original.vertices()[0].position
    );
    assert_eq!(skin.vertex_influences(0).unwrap()[0].weight, 0.5);
    let repeated = skin.evaluate(&mesh, &joints).unwrap();
    for (actual, expected) in repeated.vertices().iter().zip(original.vertices()) {
        assert_eq!(actual.position, expected.position);
        assert_eq!(actual.normal, expected.normal);
        assert_eq!(actual.uv, expected.uv);
    }
}
