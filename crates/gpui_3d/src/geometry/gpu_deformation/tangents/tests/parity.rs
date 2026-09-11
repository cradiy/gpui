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

fn underflowing_projection() -> Mesh {
    let source = projected_bitangent();
    let vertices = source
        .vertices()
        .iter()
        .copied()
        .map(|mut vertex| {
            vertex.normal = [1., 2.0_f32.powi(-80), 0.];
            vertex
        })
        .collect();
    Mesh::new(vertices, source.indices().to_vec())
}

fn underflowing_neighbor() -> Mesh {
    let base = underflowing_projection();
    let mut vertices = base.vertices().to_vec();
    for (position, uv) in [
        ([0., 0., 0.], [0., 0.]),
        ([0., 1., 0.], [0., 1.]),
        ([0., 0., 1.], [-1., 0.]),
    ] {
        vertices.push(Vertex {
            position,
            uv,
            normal: base.vertices()[0].normal,
        });
    }
    Mesh::new(vertices, (0..6).collect())
}

fn underflowing_bitangent() -> Mesh {
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([1., 1., 0.], [1., 1.]),
            ([-1., 1., 0.], [-1., 1.]),
        ]
        .into_iter()
        .map(|(position, uv)| Vertex {
            position,
            uv,
            normal: [2.0_f32.powi(-80), 1., 0.],
        })
        .collect(),
        (0..3).collect(),
    )
}

#[test]
fn cpu_projection_failure_respects_subgroups_and_encoded_tangent() {
    let generated = underflowing_neighbor()
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    assert_eq!(generated.repairs().len(), 3);
    assert!(
        generated
            .repairs()
            .iter()
            .all(|repair| repair.triangle == 0)
    );
    for &vertex in &generated.mesh().indices()[3..] {
        assert_eq!(
            generated.mesh().tangents().unwrap()[vertex as usize],
            [0., 0., -1., 1.]
        );
    }
    let generated = underflowing_bitangent()
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    let tangent = generated.mesh().tangents().unwrap()[generated.mesh().indices()[0] as usize];
    assert_eq!(tangent, [1., -2.0_f32.powi(-80), 0., 1.]);
    assert!(generated.repairs().iter().all(|repair| repair.corner != 0));
}

#[test]
fn cpu_projection_underflow_requires_explicit_basis_repair() {
    let base = underflowing_projection();
    for mode in [
        TangentGenerationMode::Strict,
        TangentGenerationMode::Inherit,
    ] {
        assert!(matches!(
            base.generate_tangents_with_mode(mode),
            Err(crate::TangentGenerationError::InvalidBasis { .. })
        ));
    }
    let generated = base
        .generate_tangents_with_mode(TangentGenerationMode::Repair)
        .unwrap();
    assert_eq!(generated.mesh().tangents().unwrap(), [[0., 0., 1., 1.]; 3]);
    assert_eq!(generated.repairs().len(), 3);
    assert!(
        generated
            .repairs()
            .iter()
            .all(|repair| repair.kind == TangentRepairKind::OrthonormalBasis)
    );
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_dynamic_projection_underflow_requires_repair_mode() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let changed = underflowing_projection();
    let base = Mesh::new(
        changed
            .vertices()
            .iter()
            .copied()
            .map(|mut vertex| {
                vertex.normal = [0., 0., 1.];
                vertex
            })
            .collect(),
        changed.indices().to_vec(),
    );
    let records = super::super::super::pack_mesh(&changed);
    let input = GpuDeformationOutput {
        context: context.clone(),
        base: base.clone(),
        buffer: buffer(
            &context.device,
            "projected tangent range",
            bytemuck::cast_slice(&records),
            wgpu::BufferUsages::STORAGE,
        ),
    };
    let frames = frames(&input, 0)?;
    for mode in [
        TangentGenerationMode::Strict,
        TangentGenerationMode::Inherit,
        TangentGenerationMode::Repair,
    ] {
        let source = GpuTangents::new(context.clone(), base.clone(), 0, mode, limits)?;
        let output = source.evaluate(&frames)?;
        if mode == TangentGenerationMode::Repair {
            assert_eq!(repair_tags(&output)?, [2; 3]);
            let (_, expected) = prepare(&changed, 0, mode)?;
            compare(&output.deformation().readback()?, &expected);
        } else {
            assert_eq!(repair_tags(&output)?, [0; 3]);
            let error = output.deformation().readback().unwrap_err().to_string();
            assert!(
                error.contains("vertex 0 failed with status [2, 0, 0, 0]"),
                "{error}"
            );
        }
    }
    Ok(())
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
    for base in [
        competing_donors(),
        projected_bitangent(),
        underflowing_projection(),
        underflowing_neighbor(),
        underflowing_bitangent(),
    ] {
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
