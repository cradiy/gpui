use anyhow::Result;
use gpui_3d::{
    Camera, HeadlessRenderer, Light, Material, Mesh, Object, PbrMaterial, Projection, ReadFrame,
    Scene, Scene3dChannels, Scene3dMaterialProgram, Scene3dMaterialSource, Scene3dMaterialValue,
    Scene3dOutputConfig, SpecularEnvironment, SpecularEnvironmentMap,
};
use std::time::{Duration, Instant};

fn environment() -> Result<SpecularEnvironment> {
    let levels = (0..3)
        .map(|level| {
            let edge = 4 >> level;
            (0..6)
                .flat_map(|face| {
                    let color = [
                        (face + 1) as f32,
                        (level + 1) as f32,
                        ((face + 1) * (level + 1)) as f32 * 0.25,
                    ];
                    vec![color; edge * edge]
                })
                .collect()
        })
        .collect();
    Ok(SpecularEnvironment::from_prefiltered(
        SpecularEnvironmentMap::from_prefiltered(4, levels)?,
    ))
}

fn read(renderer: &mut HeadlessRenderer, scene: &Scene) -> Result<ReadFrame> {
    let frame = renderer.render(
        scene,
        Scene3dOutputConfig {
            size: [33, 33],
            channels: Scene3dChannels::all(),
            color_samples: 4,
        },
    )?;
    let mut pending = frame.readback()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(pixels) = pending.try_read()? {
            return Ok(pixels);
        }
        anyhow::ensure!(Instant::now() < deadline, "environment readback timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn scene(material: Material, environment: Option<SpecularEnvironment>, tilt: f32) -> Scene {
    Scene::new()
        .camera(Camera {
            projection: Projection::Orthographic { vertical_size: 3. },
            ..Default::default()
        })
        .light(Light {
            ambient: 0.,
            ..Default::default()
        })
        .lights([])
        .specular_environment(environment)
        .object(Object::new(Mesh::plane(), material).rotation([tilt, 0., 0.]))
}

#[test]
#[ignore = "requires a GPU adapter"]
fn custom_environment_helpers_sample_faces_levels_and_inactive_inputs() -> Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let source = Scene3dMaterialSource::new(
        renderer.context().clone(),
        Scene3dMaterialProgram::compile(
            r#"
            struct Query { direction: vec4<f32>, kind: vec4<f32> }
            @group(1) @binding(0) var<uniform> query: Query;
            fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
                return builtin_surface(input, gradients);
            }
            fn material_shading(base: vec3<f32>, input: SurfaceInput,
                gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
                if (query.kind.x > 0.0) {
                    let brdf = material_environment_brdf(query.direction.x, query.direction.w);
                    return vec3<f32>(brdf, brdf.x + brdf.y);
                }
                return material_environment_radiance(query.direction.xyz, query.direction.w);
            }
            "#,
        )?,
    )?;
    let environment = environment()?;
    let half_pi = std::f32::consts::FRAC_PI_2;
    let cases = [
        ([1., 0., 0.], 0., 0., Some(1.), [1., 1., 0.25]),
        ([-1., 0., 0.], 0., 0., Some(1.), [2., 1., 0.5]),
        ([0., 1., 0.], 0., 0., Some(1.), [3., 1., 0.75]),
        ([0., -1., 0.], 0., 0., Some(1.), [4., 1., 1.]),
        ([0., 0., 1.], 0., 0., Some(1.), [5., 1., 1.25]),
        ([0., 0., -1.], 0., 0., Some(1.), [6., 1., 1.5]),
        ([0., 0., 7.], 0., half_pi, Some(1.), [2., 1., 0.5]),
        ([0., 0., 1.], 0., -half_pi, Some(1.), [1., 1., 0.25]),
        ([0., 0., 1.], 0.5, 0., Some(1.), [5., 2., 2.5]),
        ([0., 0., 1.], 0.75, 0., Some(2.), [10., 5., 6.25]),
        ([0., 0., 1.], 2., 0., Some(0.5), [2.5, 1.5, 1.875]),
        ([0., 0., 1.], -1., 0., Some(1.), [5., 1., 1.25]),
        ([0., 0., 1.], 0., 0., None, [0.; 3]),
        ([0., 0., 1.], 0., 0., Some(0.), [0.; 3]),
        ([0.; 3], 0., 0., Some(1.), [0.; 3]),
    ];
    for (index, (direction, roughness, rotation, intensity, expected)) in
        cases.into_iter().enumerate()
    {
        let data: [f32; 8] = [
            direction[0],
            direction[1],
            direction[2],
            roughness,
            0.,
            0.,
            0.,
            0.,
        ];
        let snapshot = source.bind(
            [(
                0,
                Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&data).into()),
            )],
            Default::default(),
        )?;
        let input = scene(
            Material::color(gpui::white()).program(snapshot),
            intensity.map(|value| environment.clone().rotation_y(rotation).intensity(value)),
            0.,
        );
        let result = read(&mut renderer, &input)?;
        let pixel = result.pixels.linear_rgba.as_ref().unwrap()[16 * 33 + 16];
        for (actual, expected) in pixel[..3].iter().zip(expected) {
            assert!((actual - expected).abs() < 0.003, "case {index}: {pixel:?}");
        }
        assert_eq!(pixel[3], 1.);
        assert_eq!(result.pixels.object_ids.as_ref().unwrap()[16 * 33 + 16], 1);
    }

    for intensity in [None, Some(0.), Some(1.)] {
        let data = [1_f32, 0., 0., 1., 1., 0., 0., 0.];
        let snapshot = source.bind(
            [(
                0,
                Scene3dMaterialValue::Uniform(bytemuck::cast_slice(&data).into()),
            )],
            Default::default(),
        )?;
        let input = scene(
            Material::color(gpui::white()).program(snapshot),
            intensity.map(|value| environment.clone().intensity(value)),
            0.,
        );
        let result = read(&mut renderer, &input)?;
        let pixel = result.pixels.linear_rgba.as_ref().unwrap()[16 * 33 + 16];
        if intensity == Some(1.) {
            assert!((pixel[2] - (1. - std::f32::consts::LN_2)).abs() < 0.01);
        } else {
            assert_eq!(pixel, [0., 0., 0., 1.]);
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires a GPU adapter"]
fn custom_specular_response_matches_builtin_without_changing_geometry_outputs() -> Result<()> {
    let mut renderer = HeadlessRenderer::new()?;
    let source = Scene3dMaterialSource::new(
        renderer.context().clone(),
        Scene3dMaterialProgram::compile(
            r#"
            fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
                return builtin_surface(input, gradients);
            }
            fn material_shading(base: vec3<f32>, input: SurfaceInput,
                gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> {
                let normal = surface_normal(input, gradients.normal) * face_sign;
                let view = material_view_direction(input.world);
                let factors = material_factors(input, gradients);
                let roughness = max(factors.roughness, 0.045);
                let f0 = mix(vec3<f32>(0.04), base, factors.metallic);
                let radiance = material_environment_radiance(reflect(-view, normal), roughness);
                let brdf = material_environment_brdf(dot(normal, view), roughness);
                return radiance * (f0 * brdf.x + vec3<f32>(brdf.y)) * factors.occlusion;
            }
            "#,
        )?,
    )?;
    let snapshot = source.bind([], Default::default())?;
    let environment = environment()?;
    for (roughness, metallic, tilt, rotation, intensity) in [
        (0., 1., 0., 0., Some(1.)),
        (0.25, 0., 0.4, 0., Some(1.)),
        (0.5, 0.5, 0.7, 1.2, Some(2.)),
        (1., 1., 0., 0., Some(0.5)),
        (0.75, 0., 1.1, -1.2, Some(1.)),
        (0.5, 0.5, 0.7, 0., Some(0.)),
        (0.25, 1., 0.4, 0., None),
    ] {
        let material = Material::color(gpui::rgb(0x80b0e0)).pbr(PbrMaterial {
            metallic,
            roughness,
            ..Default::default()
        });
        let environment =
            intensity.map(|value| environment.clone().rotation_y(rotation).intensity(value));
        let builtin = read(
            &mut renderer,
            &scene(material.clone(), environment.clone(), tilt),
        )?;
        let custom = read(
            &mut renderer,
            &scene(material.program(snapshot.clone()), environment, tilt),
        )?;
        assert_eq!(builtin.pixels.object_ids, custom.pixels.object_ids);
        assert_eq!(builtin.pixels.linear_depth, custom.pixels.linear_depth);
        assert_eq!(builtin.pixels.world_normals, custom.pixels.world_normals);
        for (expected, actual) in builtin
            .pixels
            .linear_rgba
            .as_ref()
            .unwrap()
            .iter()
            .zip(custom.pixels.linear_rgba.as_ref().unwrap())
        {
            for (expected, actual) in expected.iter().zip(actual) {
                assert!(
                    (actual - expected).abs() < 0.004,
                    "roughness {roughness}, tilt {tilt}: {actual} != {expected}"
                );
            }
        }
        let center = custom.pixels.linear_rgba.as_ref().unwrap()[16 * 33 + 16];
        if intensity.is_some_and(|value| value > 0.) {
            assert!(center[..3].iter().any(|value| *value > 0.01));
        } else {
            assert_eq!(center, [0., 0., 0., 1.]);
        }
    }
    Ok(())
}
