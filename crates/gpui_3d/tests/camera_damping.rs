use gpui::{Bounds, MouseButton, Pixels, point, px, size};
use gpui_3d::{Camera, OrbitController, OrbitError, OrbitSettings, Projection};
use std::time::Duration;

fn viewport() -> Bounds<Pixels> {
    Bounds::new(point(px(20.), px(40.)), size(px(800.), px(450.)))
}

fn damped(camera: Camera) -> OrbitController {
    let mut controls = OrbitController::new(camera).unwrap();
    controls
        .set_damping(Some(Duration::from_millis(80)))
        .unwrap();
    controls
}

fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 2e-5, "{a} != {b}");
}

fn radius(camera: Camera) -> f32 {
    camera
        .eye
        .iter()
        .zip(camera.target)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt()
}

#[test]
fn explicit_time_preserves_input_destination_and_settles_without_idle_frames() {
    let camera = Camera::orbit(0.4, 0.3, 8.);
    let mut controls = damped(camera);
    let mut immediate = OrbitController::new(camera).unwrap();
    assert!(!controls.advance(Duration::from_secs(20)).unwrap());
    assert!(!controls.is_animating());
    controls.orbit_by([50., 20.]).unwrap();
    immediate.orbit_by([50., 20.]).unwrap();
    assert_eq!(controls.camera(), camera);
    assert_eq!(controls.target_camera(), immediate.camera());
    assert!(controls.is_animating());
    assert!(!controls.advance(Duration::ZERO).unwrap());
    assert!(controls.advance(Duration::from_millis(80)).unwrap());
    let half = controls.camera();
    assert_ne!(half, camera);
    assert_ne!(half, controls.target_camera());
    close(radius(half), radius(camera));
    assert_eq!(half.target, camera.target);
    let mut halfway = OrbitController::new(camera).unwrap();
    halfway.orbit_by([25., 10.]).unwrap();
    for (a, b) in half.eye.into_iter().zip(halfway.camera().eye) {
        close(a, b);
    }
    controls.advance(Duration::from_secs(60)).unwrap();
    assert_eq!(controls.camera(), immediate.camera());
    assert!(!controls.is_animating());
    assert!(!controls.advance(Duration::from_secs(60)).unwrap());
}

#[test]
fn time_partitioning_and_retargeting_keep_the_same_response() {
    for projection in [
        Projection::default(),
        Projection::Orthographic { vertical_size: 5. },
    ] {
        let camera = Camera {
            projection,
            lens_shift: [0.6, -0.25],
            up: [0., 0., 2.],
            ..Camera::orbit(0.4, 0.3, 8.)
        };
        let mut coarse = damped(camera);
        let mut fine = damped(camera);
        for pass in 0..3 {
            for controls in [&mut coarse, &mut fine] {
                match pass {
                    0 => {
                        controls.orbit_by([40., -20.]).unwrap();
                    }
                    1 => {
                        controls
                            .pan_by(viewport(), point(px(40.), px(-30.)))
                            .unwrap();
                    }
                    _ => {
                        controls.dolly(0.7).unwrap();
                        controls.zoom(1.4).unwrap();
                    }
                }
            }
            coarse.advance(Duration::from_millis(80)).unwrap();
            for _ in 0..80 {
                fine.advance(Duration::from_millis(1)).unwrap();
            }
            for (a, b) in coarse.camera().eye.into_iter().zip(fine.camera().eye) {
                close(a, b);
            }
            for (a, b) in coarse.camera().target.into_iter().zip(fine.camera().target) {
                close(a, b);
            }
            assert_eq!(coarse.camera().lens_shift, camera.lens_shift);
            assert_eq!(coarse.camera().up, camera.up);
            assert_eq!(
                (coarse.camera().near, coarse.camera().far),
                (camera.near, camera.far)
            );
        }
        coarse.advance(Duration::from_secs(5)).unwrap();
        fine.advance(Duration::from_secs(5)).unwrap();
        assert_eq!(coarse.camera(), fine.camera());
    }
}

#[test]
fn optical_response_preserves_pose_and_uses_logarithmic_scale() {
    for projection in [
        Projection::Perspective { vertical_fov: 0.6 },
        Projection::Orthographic { vertical_size: 3. },
    ] {
        let camera = Camera {
            projection,
            lens_shift: [-0.3, 0.5],
            ..Camera::orbit(0.3, 0.2, 8.)
        };
        let mut controls = damped(camera);
        controls.zoom(4.).unwrap();
        controls.advance(Duration::from_millis(80)).unwrap();
        let halfway = controls.camera();
        assert_eq!(halfway.eye, camera.eye);
        assert_eq!(halfway.target, camera.target);
        let mut expected = OrbitController::new(camera).unwrap();
        expected.zoom(2.).unwrap();
        match (halfway.projection, expected.camera().projection) {
            (
                Projection::Perspective { vertical_fov: a },
                Projection::Perspective { vertical_fov: b },
            ) => close(a, b),
            (
                Projection::Orthographic { vertical_size: a },
                Projection::Orthographic { vertical_size: b },
            ) => close(a, b),
            _ => panic!("projection kind changed"),
        }
    }
}

#[test]
fn pan_follows_the_target_plane_and_dolly_preserves_optics() {
    for projection in [
        Projection::default(),
        Projection::Orthographic { vertical_size: 5. },
    ] {
        let camera = Camera {
            projection,
            lens_shift: [0.7, -0.4],
            up: [0., 0., 1.],
            ..Camera::orbit(0.3, 0.2, 8.)
        };
        let mut controls = damped(camera);
        let rect = viewport();
        let before = camera
            .world_to_screen(rect, camera.target)
            .unwrap()
            .unwrap()
            .position;
        controls.pan_by(rect, point(px(80.), px(-40.))).unwrap();
        controls.advance(Duration::from_millis(80)).unwrap();
        let after = controls
            .camera()
            .world_to_screen(rect, camera.target)
            .unwrap()
            .unwrap()
            .position;
        assert!((f32::from(after.x - before.x) - 40.).abs() < 0.001);
        assert!((f32::from(after.y - before.y) + 20.).abs() < 0.001);
        close(radius(controls.camera()), radius(camera));
        controls.cancel_drag();
        let before = controls.camera();
        controls.dolly(0.25).unwrap();
        controls.advance(Duration::from_millis(80)).unwrap();
        close(radius(controls.camera()), radius(before) * 0.5);
        assert_eq!(controls.camera().target, before.target);
        assert_eq!(controls.camera().projection, before.projection);
    }
}

#[test]
fn poles_remain_valid_with_nonstandard_up_axes() {
    for up in [[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]] {
        for sign in [-1., 1.] {
            let camera = Camera {
                eye: up.map(|v| v * sign * 6.),
                up,
                ..Camera::default()
            };
            let mut controls = damped(camera);
            controls.orbit_by([20., -sign * 40.]).unwrap();
            for _ in 0..16 {
                controls.advance(Duration::from_millis(80)).unwrap();
                controls.camera().view_projection(1.5).unwrap();
                close(radius(controls.camera()), 6.);
                assert_eq!(controls.camera().up, up);
            }
            assert!(!controls.is_animating());
            assert_eq!(controls.camera(), controls.target_camera());
        }
    }
}

#[test]
fn release_continues_motion_but_cancellation_and_new_gestures_freeze_it() {
    let rect = viewport();
    let start = rect.center();
    let mut controls = damped(Camera::default());
    controls
        .begin_drag(MouseButton::Right, start, rect)
        .unwrap();
    assert!(!controls.is_animating());
    controls
        .update_drag(
            start + point(px(50.), px(20.)),
            Some(MouseButton::Right),
            rect,
        )
        .unwrap();
    assert!(!controls.end_drag(MouseButton::Left));
    assert!(controls.is_dragging());
    assert!(controls.end_drag(MouseButton::Right));
    assert!(controls.is_animating());
    controls.advance(Duration::from_millis(20)).unwrap();
    let before = controls.camera();
    assert!(
        controls
            .begin_drag(MouseButton::Middle, start, rect)
            .unwrap()
    );
    assert_eq!(controls.target_camera(), before);
    assert!(!controls.is_animating());
    controls
        .update_drag(
            start + point(px(20.), px(0.)),
            Some(MouseButton::Middle),
            rect,
        )
        .unwrap();
    controls.advance(Duration::from_millis(20)).unwrap();
    let before = controls.camera();
    assert!(!controls.update_drag(start, None, rect).unwrap());
    assert_eq!(controls.camera(), before);
    assert_eq!(controls.target_camera(), before);
    assert!(!controls.is_animating());
    assert!(!controls.is_dragging());
}

#[test]
fn configuration_is_atomic_and_large_durations_reach_idle() {
    let mut controls = damped(Camera::default());
    controls.scroll(100.).unwrap();
    controls.advance(Duration::from_millis(20)).unwrap();
    let before = controls.camera();
    let destination = controls.target_camera();
    assert_eq!(
        controls.set_damping(Some(Duration::ZERO)),
        Err(OrbitError::InvalidSettings)
    );
    assert!(controls.orbit_by([f32::NAN, 0.]).is_err());
    assert!(
        controls
            .set_camera(Camera {
                eye: [0.; 3],
                ..Camera::default()
            })
            .is_err()
    );
    assert_eq!(controls.camera(), before);
    assert_eq!(controls.target_camera(), destination);
    assert!(controls.is_animating());
    controls.set_damping(None).unwrap();
    assert_eq!(controls.camera(), before);
    assert!(!controls.is_animating());
    controls.scroll(100.).unwrap();
    assert_ne!(controls.camera(), before);
    assert_eq!(controls.camera(), controls.target_camera());
    controls.set_damping(Some(Duration::MAX)).unwrap();
    controls.scroll(-50.).unwrap();
    for _ in 0..16 {
        controls.advance(Duration::MAX).unwrap();
    }
    assert!(!controls.is_animating());
    assert_eq!(controls.camera(), controls.target_camera());
    controls.orbit_by([20., 5.]).unwrap();
    controls.set_camera(Camera::default()).unwrap();
    assert!(!controls.is_animating());
    assert_eq!(controls.camera(), Camera::default());
}

#[test]
fn limits_and_shortest_orbit_path_do_not_cross_the_target() {
    let mut controls = damped(Camera::orbit(170_f32.to_radians(), 0., 6.));
    controls
        .orbit_by([-20_f32.to_radians() / controls.settings().orbit_speed, 0.])
        .unwrap();
    controls.advance(Duration::from_millis(80)).unwrap();
    close(radius(controls.camera()), 6.);
    assert!(controls.camera().eye[2] < -5.9);
    controls.set_camera(Camera::default()).unwrap();
    controls
        .set_settings(OrbitSettings {
            pitch: -0.4..=0.5,
            distance: 2.0..=8.,
            ..Default::default()
        })
        .unwrap();
    controls.orbit_by([0., 1000.]).unwrap();
    controls.dolly(0.0001).unwrap();
    for _ in 0..160 {
        controls.advance(Duration::from_millis(8)).unwrap();
        let camera = controls.camera();
        assert!(radius(camera) >= 2. - 1e-5 && radius(camera) <= 6. + 1e-5);
        assert!((camera.eye[1] / radius(camera)).asin() <= 0.5 + 1e-5);
        camera.view_projection(1.5).unwrap();
    }
    close(radius(controls.camera()), 2.);
}
