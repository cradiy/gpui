use super::*;
use crate::TangentGenerationError;

fn scaled_triangle(position_scale: f32, uv_scale: f32) -> Mesh {
    Mesh::new(
        [
            ([0., 0., 0.], [0., 0.]),
            ([position_scale, 0., 0.], [uv_scale, 0.]),
            ([0., position_scale, 0.], [0., uv_scale]),
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

fn thin_triangle() -> Mesh {
    let mut vertices = scaled_triangle(1., 1.).vertices().to_vec();
    vertices[1].position = [1. + f32::EPSILON, 1., 0.];
    vertices[2].position = [1. + 2. * f32::EPSILON, 1. + f32::EPSILON, 0.];
    Mesh::new(vertices, vec![0, 1, 2])
}

#[test]
fn cpu_thin_triangle_retains_nonzero_area_and_usable_frames() {
    let base = thin_triangle();
    let x = f64::from(1. + f32::EPSILON);
    let length = x.hypot(1.);
    let expected = [(x / length) as f32, (1. / length) as f32, 0., 1.];
    for mode in [
        TangentGenerationMode::Strict,
        TangentGenerationMode::Inherit,
        TangentGenerationMode::Repair,
    ] {
        let generated = base.generate_tangents_with_mode(mode).unwrap();
        assert!(generated.repairs().is_empty());
        assert_eq!(generated.mesh().indices(), base.indices());
        for (generated, original) in generated.mesh().vertices().iter().zip(base.vertices()) {
            assert_eq!(generated.position, original.position);
        }
        for tangent in generated.mesh().tangents().unwrap() {
            for (actual, expected) in tangent.iter().zip(expected) {
                assert!((actual - expected).abs() < 1e-7);
            }
        }
    }
}

#[test]
#[ignore = "requires a compute-capable GPU with SHADER_F64"]
fn gpu_thin_triangle_retains_area_classification_during_publication() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    let base = scaled_triangle(1., 1.);
    let changed = thin_triangle();
    let records = super::super::super::pack_mesh(&changed);
    let input = GpuDeformationOutput {
        context: context.clone(),
        base: base.clone(),
        buffer: context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("thin tangent triangle"),
            contents: bytemuck::cast_slice(&records),
            usage: wgpu::BufferUsages::STORAGE,
        }),
    };
    let frames = frames(&input, 0)?;
    for mode in [
        TangentGenerationMode::Strict,
        TangentGenerationMode::Inherit,
        TangentGenerationMode::Repair,
    ] {
        let source = GpuTangents::new(context.clone(), base.clone(), 0, mode, limits)?;
        let output = source.evaluate(&frames)?;
        assert_eq!(repair_tags(&output)?, [0; 3]);
        let (_, expected) = prepare(&changed, 0, mode)?;
        compare(&output.deformation().readback()?, &expected);
    }
    Ok(())
}

#[test]
fn cpu_repair_does_not_override_tangent_numeric_admission() {
    for (position, uv) in [
        (2f32.powi(65), 1.),
        (2f32.powi(-65), 1.),
        (2f32.powi(33), 2f32.powi(33)),
        (2f32.powi(32), 2f32.powi(-96)),
    ] {
        let base = scaled_triangle(position, uv);
        for mode in [
            TangentGenerationMode::Strict,
            TangentGenerationMode::Inherit,
            TangentGenerationMode::Repair,
        ] {
            assert!(
                matches!(
                    base.generate_tangents_with_mode(mode),
                    Err(TangentGenerationError::Unrepresentable { triangle: 0 })
                ),
                "{position}, {uv}, {mode:?}"
            );
        }
    }
    for scale in [2f32.powi(-62), 1., 2f32.powi(62)] {
        assert!(
            scaled_triangle(scale, 1.)
                .generate_tangents_with_mode(TangentGenerationMode::Strict)
                .is_ok()
        );
    }
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn gpu_tangent_publication_enforces_numeric_domain_in_every_mode() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let limits = GpuDeformationLimits::default();
    for mode in [
        TangentGenerationMode::Strict,
        TangentGenerationMode::Inherit,
        TangentGenerationMode::Repair,
    ] {
        for (changed, uv, rejected) in [
            (2f32.powi(65), 1., true),
            (2f32.powi(-65), 1., true),
            (2f32.powi(33), 2f32.powi(33), true),
            (2f32.powi(-62), 1., false),
            (1., 1., false),
            (2f32.powi(62), 1., false),
        ] {
            let base = scaled_triangle(1., uv);
            let publisher = GpuTangents::new(context.clone(), base.clone(), 0, mode, limits)?;
            let changed = scaled_triangle(changed, uv);
            let records = super::super::super::pack_mesh(&changed);
            let input = GpuDeformationOutput {
                context: context.clone(),
                base,
                buffer: context.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tangent numeric range"),
                    contents: bytemuck::cast_slice(&records),
                    usage: wgpu::BufferUsages::STORAGE,
                }),
            };
            let output = publisher.evaluate(&frames(&input, 0)?)?;
            assert_eq!(repair_tags(&output)?, [0; 3]);
            if rejected {
                let error = output.deformation().readback().unwrap_err().to_string();
                assert!(
                    error.contains("vertex 0 failed with status [5, 0, 0, 0]"),
                    "{error}"
                );
            } else {
                let (_, expected) = prepare(&changed, 0, mode)?;
                compare(&output.deformation().readback()?, &expected);
            }
        }
    }
    Ok(())
}
