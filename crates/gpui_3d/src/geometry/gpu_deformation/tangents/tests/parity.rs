use super::*;
use crate::TangentRepairKind;

fn projected_bitangent() -> Mesh {
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([1., 0., 0.], [1., 0.]),
            ([0., 1., 0.], [0., 1.]),
        ]
        .into_iter()
        .map(|(position, uv)| Vertex {
            position,
            normal: [0., 1., 0.],
            uv,
        })
        .collect(),
        (0..3).collect(),
    )
}

#[test]
fn cpu_projection_preserves_tangents_when_only_bitangent_is_zero() {
    let generated = projected_bitangent()
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    assert_eq!(generated.mesh().tangents().unwrap(), [[1., 0., 0., 1.]; 3]);
    assert_eq!(
        generated
            .repairs()
            .iter()
            .map(|repair| (repair.triangle, repair.corner, repair.kind))
            .collect::<Vec<_>>(),
        [(0, 1, TangentRepairKind::TriangleDerivative)]
    );
}

fn competing_donors() -> Mesh {
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([1., 0., 0.], [1., 0.]),
            ([0., 1., 0.], [0., 1.]),
            ([0., 0., 0.], [0., 0.]),
            ([0., 0., 1.], [1., 0.]),
            ([0., 1., 0.], [0., 1.]),
            ([0., 0., 0.], [0., 0.]),
            ([1., 0., 0.], [1., 0.]),
            ([1., 0., 0.], [1., 0.]),
        ]
        .into_iter()
        .map(|(position, uv)| Vertex {
            position,
            normal: [1., 0., 0.],
            uv,
        })
        .collect(),
        (0..9).collect(),
    )
}

#[test]
fn cpu_inheritance_preserves_undefined_first_donor_until_explicit_repair() {
    let base = competing_donors();
    let generated = base
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    let tangents = generated.mesh().tangents().unwrap();
    let at_corner = |corner| tangents[generated.mesh().indices()[corner] as usize];
    assert_eq!(at_corner(0), [0., 1., 0., 1.]);
    assert_eq!(at_corner(3), [0., 0., 1., 1.]);
    assert_eq!(at_corner(6), at_corner(0));
    assert_ne!(at_corner(6), at_corner(3));
    let repaired = generated
        .repairs()
        .iter()
        .filter(|repair| repair.triangle == 2)
        .map(|repair| (repair.corner, repair.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        repaired,
        (0..3)
            .map(|corner| (corner, TangentRepairKind::OrthonormalBasis))
            .collect::<Vec<_>>()
    );
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_frames_match_cpu_repair_selection_for_inheritance_and_projection() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    for base in [competing_donors(), projected_bitangent()] {
        let source = GpuTangents::new(
            context.clone(),
            base.clone(),
            0,
            TangentGenerationMode::Repair,
            limits,
        )?;
        let input = GpuDeformationOutput::upload(context.clone(), base.clone(), limits)?;
        let output = source.evaluate(&frames(&input, 0)?)?;
        drop((source, input));
        let expected = base.generate_tangents_with_mode(TangentGenerationMode::Repair)?;
        let mut expected_tags = vec![0; base.vertex_count()];
        for repair in expected.repairs() {
            let vertex = base.indices()[repair.triangle * 3 + repair.corner] as usize;
            expected_tags[vertex] = match repair.kind {
                TangentRepairKind::TriangleDerivative => 1,
                TangentRepairKind::OrthonormalBasis => 2,
            };
        }
        assert_eq!(repair_tags(&output)?, expected_tags);
        let (_, expected) = prepare(&base, 0, TangentGenerationMode::Repair)?;
        compare(&output.deformation().readback()?, &expected);
    }
    Ok(())
}
