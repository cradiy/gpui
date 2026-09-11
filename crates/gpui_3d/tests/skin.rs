use gpui_3d::{AffineTransform, Mesh, NormalizedSkinInfluence, Skin, SkinError, SkinInfluence};

fn influence(joint: usize, weight: f32) -> SkinInfluence {
    SkinInfluence { joint, weight }
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
