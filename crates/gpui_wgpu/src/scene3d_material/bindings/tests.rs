use super::*;
use plan::Input;

fn program() -> MaterialProgram {
    MaterialProgram::compile(&format!(
        r#"
        struct Small {{ value: vec4<f32> }}
        struct Large {{ value: array<vec4<f32>, 2> }}
        @group(1) @binding(1) var<uniform> first: Small;
        @group(1) @binding(9) var<uniform> second: Large;
        @group(1) @binding(2) var picture: texture_2d<f32>;
        @group(1) @binding(4) var picture_sampler: sampler;
        {}
    "#,
        super::super::DEFAULT
    ))
    .unwrap()
}

#[test]
fn material_bindings_preserve_order_and_budget_complete_and_partial_snapshots() {
    let program = program();
    let limits = Scene3dMaterialBindingLimits {
        max_uniform_bytes: 48,
    };
    let inputs = [
        (9, Input::Uniform(32)),
        (4, Input::Sampler),
        (1, Input::Uniform(16)),
        (2, Input::Texture),
    ];
    let (mapping, bytes) = plan::resolve(program.resources(), inputs, false, limits).unwrap();
    assert_eq!(mapping, [Some(2), Some(3), Some(1), Some(0)]);
    assert_eq!(bytes, 48);
    let (mapping, bytes) =
        plan::resolve(program.resources(), [(1, Input::Uniform(16))], true, limits).unwrap();
    assert_eq!(mapping, [Some(0), None, None, None]);
    assert_eq!(bytes, 48);
    assert!(
        plan::resolve(
            program.resources(),
            [],
            true,
            Scene3dMaterialBindingLimits {
                max_uniform_bytes: 47
            }
        )
        .is_err()
    );
    let (mapping, _) = plan::resolve(program.resources(), [], true, limits).unwrap();
    assert!(mapping.iter().all(Option::is_none));
}

#[test]
fn material_bindings_reject_missing_duplicate_unknown_and_mistyped_values() {
    let program = program();
    let limits = Scene3dMaterialBindingLimits::default();
    assert!(plan::resolve(program.resources(), [], false, limits).is_err());
    for (inputs, message) in [
        (
            vec![(1, Input::Uniform(16)), (1, Input::Uniform(16))],
            "duplicate",
        ),
        (vec![(3, Input::Uniform(16))], "unknown"),
        (vec![(1, Input::Texture)], "incompatible"),
        (vec![(2, Input::Sampler)], "incompatible"),
        (vec![(4, Input::Uniform(16))], "incompatible"),
        (vec![(1, Input::Uniform(15))], "exactly 16"),
        (vec![(1, Input::Uniform(17))], "exactly 16"),
    ] {
        let error = plan::resolve(program.resources(), inputs, true, limits).unwrap_err();
        assert!(error.to_string().contains(message), "{error:#}");
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn material_binding_snapshots_retain_uniform_buffers_and_validate_gpu_resources() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let source = Scene3dMaterialSource::new(context.clone(), program())?;
    let texture = context.create_texture(&wgpu::TextureDescriptor {
        label: Some("material input"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let sampler = context.create_sampler(&Default::default());
    let limits = Scene3dMaterialBindingLimits::default();
    let original = source.bind(
        [
            (1, Scene3dMaterialValue::Uniform(vec![0; 16].into())),
            (9, Scene3dMaterialValue::Uniform(vec![0; 32].into())),
            (2, Scene3dMaterialValue::Texture(view)),
            (4, Scene3dMaterialValue::Sampler(sampler)),
        ],
        limits,
    )?;
    let changed = original.with_values(
        [(1, Scene3dMaterialValue::Uniform(vec![1; 16].into()))],
        limits,
    )?;
    assert_eq!(changed.uniform_bytes(), 48);
    assert_eq!(changed.source().layout(), original.source().layout());
    assert_eq!(changed.source().shader(), original.source().shader());
    assert_ne!(changed.bind_group(), original.bind_group());
    for (index, equal) in [(0, false), (3, true)] {
        let BoundValue::Uniform { buffer: a } = &original.0.slots[index] else {
            panic!("uniform slot")
        };
        let BoundValue::Uniform { buffer: b } = &changed.0.slots[index] else {
            panic!("uniform slot")
        };
        assert_eq!(a == b, equal);
        assert_eq!(a.usage(), wgpu::BufferUsages::UNIFORM);
    }
    let comparison = context.create_sampler(&wgpu::SamplerDescriptor {
        compare: Some(wgpu::CompareFunction::Less),
        ..Default::default()
    });
    assert!(
        original
            .with_values([(4, Scene3dMaterialValue::Sampler(comparison))], limits)
            .is_err()
    );
    let wrong_view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    assert!(
        original
            .with_values([(2, Scene3dMaterialValue::Texture(wrong_view))], limits)
            .is_err()
    );
    for (format, usage, sample_count) in [
        (
            wgpu::TextureFormat::R32Uint,
            wgpu::TextureUsages::TEXTURE_BINDING,
            1,
        ),
        (
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::COPY_DST,
            1,
        ),
        (
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            4,
        ),
    ] {
        let incompatible = context.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: texture.size(),
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        });
        let error = original
            .with_values(
                [(
                    2,
                    Scene3dMaterialValue::Texture(incompatible.create_view(&Default::default())),
                )],
                limits,
            )
            .err()
            .expect("incompatible material texture accepted");
        assert!(error.to_string().contains("material binding"), "{error:#}");
    }
    let foreign = WgpuContext::new_headless()?;
    let foreign_sampler = foreign.create_sampler(&Default::default());
    assert!(
        original
            .with_values(
                [(4, Scene3dMaterialValue::Sampler(foreign_sampler))],
                limits
            )
            .err()
            .unwrap()
            .to_string()
            .contains("different device")
    );
    let foreign_texture = foreign.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: texture.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let foreign_view = foreign_texture.create_view(&Default::default());
    assert!(
        original
            .with_values([(2, Scene3dMaterialValue::Texture(foreign_view))], limits)
            .err()
            .unwrap()
            .to_string()
            .contains("different device")
    );
    drop(source);
    drop(texture);
    drop(context);
    let retained = original.with_values([], limits)?;
    assert_eq!(retained.bind_group(), original.bind_group());
    assert_eq!(retained.source().layout(), changed.source().layout());
    assert_eq!(retained.uniform_bytes(), original.uniform_bytes());
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn material_snapshot_updates_reject_reported_device_loss() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let program = MaterialProgram::compile(super::super::DEFAULT)?;
    let source = Scene3dMaterialSource::new(context.clone(), program.clone())?;
    let limits = Scene3dMaterialBindingLimits::default();
    let original = source.bind([], limits)?;
    context
        .device_lost_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    for error in [
        source.bind([], limits).err(),
        original.with_values([], limits).err(),
        Scene3dMaterialSource::new(context, program.clone()).err(),
    ] {
        let error = error.expect("reported device loss must reject material publication");
        assert!(
            error.to_string().contains("material device is lost"),
            "{error:#}"
        );
    }
    let replacement = Scene3dMaterialSource::new(WgpuContext::new_headless()?, program)?;
    replacement.bind([], limits)?;
    Ok(())
}
