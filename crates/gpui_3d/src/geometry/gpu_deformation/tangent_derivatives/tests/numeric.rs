use super::*;
use crate::{TangentGenerationError, TangentGenerationMode, TangentRepairKind};

fn triangle(position_scale: f32, uv_scale: [f32; 2]) -> Mesh {
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([position_scale, 0., 0.], [uv_scale[0], 0.]),
            ([0., position_scale, 0.], [0., uv_scale[1]]),
        ]
        .into_iter()
        .map(|(position, uv)| Vertex {
            position,
            normal: [0., 0., 1.],
            uv,
        })
        .collect(),
        vec![0, 1, 2],
    )
}

fn cases() -> [(Mesh, bool); 4] {
    let tiny = 2f32.powi(-63);
    let large = 2f32.powi(63);
    [
        (triangle(2f32.powi(62), [tiny, tiny]), true),
        (triangle(2f32.powi(62), [tiny, tiny * 2.]), false),
        (triangle(tiny, [large, large]), true),
        (triangle(tiny * 2., [large, large]), false),
    ]
}

#[test]
fn cpu_normal_range_boundaries_distinguish_undefined_frames_from_invalid_arithmetic() {
    for (base, undefined) in cases() {
        for mirrored in [false, true] {
            let base = if mirrored {
                base.with_uv_set(
                    0,
                    base.vertices()
                        .iter()
                        .map(|v| [v.uv[0], -v.uv[1]])
                        .collect(),
                )
                .unwrap()
            } else {
                base.clone()
            };
            let strict = base.generate_tangents_with_mode(TangentGenerationMode::Strict);
            if undefined {
                assert!(
                    matches!(strict, Err(TangentGenerationError::InvalidBasis { .. })),
                    "{strict:?}"
                );
            } else {
                assert!(strict.is_ok(), "{strict:?}");
            }
            let repaired = base
                .generate_tangents_with_mode(TangentGenerationMode::Repair)
                .unwrap();
            assert_eq!(repaired.repairs().len(), if undefined { 3 } else { 0 });
            assert!(
                repaired
                    .repairs()
                    .iter()
                    .all(|r| r.kind == TangentRepairKind::TriangleDerivative)
            );
            for tangent in repaired.mesh().tangents().unwrap() {
                assert_eq!(*tangent, [1., 0., 0., if mirrored { -1. } else { 1. }]);
            }
        }
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_derivative_eligibility_matches_cpu_at_normal_range_boundaries() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    for (base, undefined) in cases() {
        for mirrored in [false, true] {
            let base = if mirrored {
                base.with_uv_set(
                    0,
                    base.vertices()
                        .iter()
                        .map(|v| [v.uv[0], -v.uv[1]])
                        .collect(),
                )?
            } else {
                base.clone()
            };
            let derivatives = GpuTangentDerivatives::new(context.clone(), base.clone(), 0, limits)?;
            let input = GpuDeformationOutput::upload(context.clone(), base, limits)?;
            let output = derivatives.evaluate(&input)?;
            drop((derivatives, input));
            let record = read(&output)?[0];
            assert_eq!(record.status, [0; 4]);
            assert_eq!(
                record.classification,
                [0, 0, u32::from(!mirrored), u32::from(undefined)]
            );
            if undefined {
                assert_eq!(record.tangent, [0.; 4]);
                assert_eq!(record.bitangent, [0.; 4]);
            } else {
                assert_eq!(record.tangent[..3], [1., 0., 0.]);
                assert_eq!(
                    record.bitangent[..3],
                    [0., if mirrored { -1. } else { 1. }, 0.]
                );
                assert!(record.tangent[3] > f32::MIN_POSITIVE);
                assert!(record.bitangent[3] > f32::MIN_POSITIVE);
            }
        }
    }
    Ok(())
}
