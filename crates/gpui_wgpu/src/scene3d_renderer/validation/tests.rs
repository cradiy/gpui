use super::*;
use crate::wgpu_renderer::scene3d::tests::frame;
use gpui::{DirectionalShadow3d, LightKind3d, PunctualLight3d};

mod objects;

#[test]
fn shadow_admission_tracks_selected_light_and_device_extent() {
    let mut input = frame(&[]);
    input.directional_shadow = Some(DirectionalShadow3d {
        light_index: 0,
        view_projection: input.view_projection,
        resolution: 1024,
        depth_bias: 0.002,
        normal_bias: 0.01,
        softness: 1.,
    });
    assert!(validate_frame_settings(&input, 1024, true).is_ok());
    assert!(validate_frame_settings(&input, 512, true).is_err());
    assert!(validate_frame_settings(&input, 512, false).is_err());
    input.directional_shadow.as_mut().unwrap().resolution = 0;
    assert!(validate_frame_settings(&input, 1024, true).is_err());
    input.directional_shadow.as_mut().unwrap().resolution = 1024;
    input.directional_shadow.as_mut().unwrap().depth_bias = f32::NAN;
    assert!(validate_frame_settings(&input, 1024, true).is_err());
    input.directional_shadow.as_mut().unwrap().depth_bias = 0.002;
    input.light_direction = [0.; 3];
    assert!(validate_frame_settings(&input, 1024, true).is_err());
    input.light_direction = [0., 0., 1.];

    let directional = PunctualLight3d {
        kind: LightKind3d::Directional,
        position: [0.; 3],
        direction: [0., 0., 1.],
        color: gpui::rgb(0xffffff),
        intensity: 1.,
        range: None,
        minimum_distance: 0.01,
        inner_angle: 0.,
        outer_angle: 1.,
    };
    input.lights = Some(
        vec![
            PunctualLight3d {
                kind: LightKind3d::Point,
                ..directional
            },
            directional,
        ]
        .into(),
    );
    assert!(validate_frame_settings(&input, 1024, true).is_err());
    input.directional_shadow.as_mut().unwrap().light_index = 1;
    assert!(validate_frame_settings(&input, 1024, true).is_ok());
    input.directional_shadow.as_mut().unwrap().light_index = 2;
    assert!(validate_frame_settings(&input, 1024, true).is_err());

    input.directional_shadow = None;
    let mut lights = vec![directional; gpui::MAX_PUNCTUAL_LIGHTS_3D];
    input.lights = Some(lights.clone().into());
    assert!(validate_frame_settings(&input, 1024, true).is_ok());
    lights.push(directional);
    input.lights = Some(lights.into());
    assert!(validate_frame_settings(&input, 1024, true).is_err());
    input.lights = Some(
        vec![PunctualLight3d {
            intensity: -1.,
            ..directional
        }]
        .into(),
    );
    assert!(validate_frame_settings(&input, 1024, true).is_err());
}

#[test]
fn nonfinite_frame_uniforms_and_zero_orthographic_direction_are_rejected() {
    let edits: &[fn(&mut Scene3dFrame)] = &[
        |frame| frame.view_projection[0][0] = f32::NAN,
        |frame| frame.world_to_view[2][3] = f32::INFINITY,
        |frame| frame.camera_position[1] = f32::NEG_INFINITY,
        |frame| frame.orthographic_view_direction = Some([0.; 3]),
        |frame| frame.orthographic_view_direction = Some([f32::NAN, 1., 0.]),
        |frame| frame.light_direction[0] = f32::INFINITY,
        |frame| frame.light[3] = f32::NAN,
        |frame| frame.ambient = f32::NAN,
        |frame| frame.color_output.exposure = f32::NAN,
        |frame| frame.color_output.exposure = 17.,
    ];
    for edit in edits {
        let mut input = frame(&[]);
        assert!(validate_frame_settings(&input, 1024, true).is_ok());
        edit(&mut input);
        assert!(validate_frame_settings(&input, 1024, true).is_err());
        assert!(validate_frame_settings(&input, 1024, false).is_err());
    }
    let mut input = frame(&[]);
    input.orthographic_view_direction = Some([0., 0., 1.]);
    assert!(validate_frame_settings(&input, 1024, false).is_ok());
}

#[test]
fn environment_admission_respects_shaded_outputs_and_texture_limits() {
    let mut input = frame(&[]);
    input.background = Some(gpui::EnvironmentBackground3d {
        map: gpui::EnvironmentMap3d::from_equirectangular([4, 2], vec![[1.; 3]; 8]).unwrap(),
        intensity: 1.,
        rotation_y: 0.,
        rays: [[0., 0., -1.], [1., 0., 0.], [0., 1., 0.]],
    });
    assert!(validate_frame_settings(&input, 4, true).is_ok());
    assert!(validate_frame_settings(&input, 2, true).is_err());
    assert!(validate_frame_settings(&input, 2, false).is_ok());
    input.background.as_mut().unwrap().rays[0] = [0.; 3];
    assert!(validate_frame_settings(&input, 4, true).is_err());
    assert!(validate_frame_settings(&input, 4, false).is_ok());
    input.background = None;

    input.specular_environment = Some(gpui::SpecularEnvironment3d {
        map: gpui::SpecularEnvironmentMap3d::from_prefiltered(
            4,
            vec![vec![[1.; 3]; 96], vec![[1.; 3]; 24], vec![[1.; 3]; 6]],
        )
        .unwrap(),
        intensity: 1.,
        rotation_y: 0.,
    });
    assert!(validate_frame_settings(&input, 4, true).is_ok());
    assert!(validate_frame_settings(&input, 2, true).is_err());
    assert!(validate_frame_settings(&input, 2, false).is_ok());
    input.specular_environment.as_mut().unwrap().intensity = f32::NAN;
    assert!(validate_frame_settings(&input, 4, true).is_err());
    assert!(validate_frame_settings(&input, 4, false).is_ok());

    input.diffuse_environment = Some(gpui::DiffuseEnvironment3d {
        coefficients: [[f32::NAN; 3]; 9],
        intensity: 1.,
        rotation_y: 0.,
    });
    assert!(validate_frame_settings(&input, 4, false).is_err());
}
