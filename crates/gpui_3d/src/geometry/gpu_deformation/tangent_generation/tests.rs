use super::*;

mod parity;

#[test]
fn combined_admission_covers_retained_sources_and_transient_stages() {
    for corners in [3, 6, 63, 66, 129, 4098] {
        let memory =
            GpuTangentGenerationMemory::plan(corners, GpuDeformationLimits::default()).unwrap();
        let limits = GpuDeformationLimits {
            max_source_bytes: memory.source_bytes,
            max_output_bytes: memory.evaluation_bytes,
        };
        assert_eq!(
            GpuTangentGenerationMemory::plan(corners, limits).unwrap(),
            memory
        );
        assert!(
            GpuTangentGenerationMemory::plan(
                corners,
                GpuDeformationLimits {
                    max_source_bytes: limits.max_source_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert!(
            GpuTangentGenerationMemory::plan(
                corners,
                GpuDeformationLimits {
                    max_output_bytes: limits.max_output_bytes - 1,
                    ..limits
                }
            )
            .is_err()
        );
        assert_eq!(memory.retained_output_bytes, corners as u64 * 68);
        let publication_only = GpuTangentsMemory::plan(corners, limits).unwrap();
        assert!(
            GpuTangentGenerationMemory::plan(
                corners,
                GpuDeformationLimits {
                    max_output_bytes: publication_only.vertex_bytes + publication_only.repair_bytes,
                    ..limits
                }
            )
            .is_err()
        );
    }
    for corners in [0, 1, 4, (1 << 30) + 1, usize::MAX] {
        assert!(
            GpuTangentGenerationMemory::plan(corners, GpuDeformationLimits::default()).is_err()
        );
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn composed_generation_retains_morph_snapshots_and_mesh_identity() -> Result<()> {
    use crate::{GpuMorph, MorphTarget, MorphTargets};
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let original = Mesh::plane();
    let targets = MorphTargets::new(
        original.clone(),
        [MorphTarget {
            positions: Some(
                vec![[0., 0., 0.], [0., 0., 0.2], [0., 0., 0.7], [0., 0., -0.1]].into(),
            ),
            ..Default::default()
        }],
    )?;
    let expanded = original.expand_corners(6)?;
    let targets = targets.remap_vertices(expanded.mesh().clone(), expanded.source_vertices())?;
    let morph = GpuMorph::new(context.clone(), targets.clone(), limits)?;
    let generator = GpuTangentGeneration::new(
        context.clone(),
        expanded.mesh().clone(),
        0,
        TangentGenerationMode::Strict,
        limits,
    )?;
    assert_eq!(generator.uv_set(), 0);
    assert!(std::sync::Arc::ptr_eq(
        &generator.base_mesh().0,
        &expanded.mesh().0
    ));
    let foreign = GpuDeformationOutput::upload(context, original, limits)?;
    assert!(generator.evaluate(&foreign).is_err());
    let mut outputs = Vec::new();
    for weight in [-0.5, 1., 0.] {
        let input = morph.evaluate(&[weight])?;
        let output = generator.evaluate(&input)?;
        assert!(std::sync::Arc::ptr_eq(
            &output.deformation().base_mesh().0,
            &generator.output_mesh().0
        ));
        assert_eq!(
            output.deformation().buffer().size() + output.repair_buffer().size(),
            generator.memory().retained_output_bytes
        );
        outputs.push((weight, output));
    }
    drop((generator, morph));
    for (weight, output) in outputs {
        let expected = targets
            .evaluate(&[weight])?
            .generate_tangents_with_mode(TangentGenerationMode::Strict)?;
        let actual = output.deformation().readback()?;
        assert_eq!(actual.indices(), expanded.mesh().indices());
        for (index, &source) in expected.source_vertices().iter().enumerate() {
            let source = source as usize;
            for (a, b) in actual.vertices()[source]
                .position
                .into_iter()
                .zip(expected.mesh().vertices()[index].position)
            {
                assert!((a - b).abs() < 1e-5);
            }
            for (a, b) in actual.tangents().unwrap()[source]
                .into_iter()
                .zip(expected.mesh().tangents().unwrap()[index])
            {
                assert!((a - b).abs() < 1e-5, "{a} != {b}");
            }
        }
    }
    Ok(())
}
