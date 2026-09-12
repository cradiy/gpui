use gpui::{Bounds, Pixels, point, px, size};
use gpui_3d::{Aabb, Camera, CameraError, Projection};

fn camera() -> Camera {
    Camera {
        eye: [0.; 3],
        target: [0., 0., -1.],
        projection: Projection::Perspective {
            vertical_fov: std::f32::consts::FRAC_PI_2,
        },
        near: 1.,
        far: 10.,
        ..Default::default()
    }
}

fn viewport() -> Bounds<Pixels> {
    Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)))
}

fn coordinates(bounds: Bounds<Pixels>) -> [f32; 4] {
    [
        bounds.origin.x.into(),
        bounds.origin.y.into(),
        bounds.right().into(),
        bounds.bottom().into(),
    ]
}

fn assert_rect(actual: Option<Bounds<Pixels>>, expected: [f32; 4]) {
    let actual = coordinates(actual.expect("nonempty clipped intersection"));
    for (a, b) in actual.into_iter().zip(expected) {
        assert!((a - b).abs() < 1e-3, "{actual:?} != {expected:?}");
    }
}

#[test]
fn projection_clips_near_eye_and_side_crossings() {
    let camera = camera();
    for (min, max, expected) in [
        ([-1., -1., -4.], [1., 1., -2.], [25., 25., 75., 75.]),
        (
            [-0.5, -0.25, -2.],
            [0.5, 0.25, -0.5],
            [25., 37.5, 75., 62.5],
        ),
        ([-0.5, -0.5, -2.], [0.5, 0.5, 0.5], [25., 25., 75., 75.]),
        ([1., -1., -4.], [3., 1., -2.], [62.5, 25., 100., 75.]),
    ] {
        assert_rect(
            camera
                .project_bounds(viewport(), Aabb::new(min, max).unwrap())
                .unwrap(),
            expected,
        );
    }
    let orthographic = Camera {
        projection: Projection::Orthographic { vertical_size: 4. },
        ..camera
    };
    for depth in [-2., -8.] {
        assert_rect(
            orthographic
                .project_bounds(
                    viewport(),
                    Aabb::new([-1., -1., depth], [1., 1., depth]).unwrap(),
                )
                .unwrap(),
            [25., 25., 75., 75.],
        );
    }
}

#[test]
fn closed_clip_volume_retains_contacts_and_rejects_separated_boxes() {
    let camera = camera();
    let frustum = camera.frustum(1.).unwrap();
    for (min, max) in [
        ([-1., -1., 1.], [1., 1., 2.]),
        ([-1., -1., -12.], [1., 1., -11.]),
        ([-0.1, -0.1, -0.5], [0.1, 0.1, -0.1]),
        ([20., -1., -4.], [21., 1., -2.]),
    ] {
        let bounds = Aabb::new(min, max).unwrap();
        assert!(!frustum.intersects(bounds));
        assert_eq!(frustum.project_bounds(viewport(), bounds).unwrap(), None);
    }
    for depth in [-1., -2., -10.] {
        let bounds = Aabb::new([0., 0., depth], [0., 0., depth]).unwrap();
        assert!(frustum.intersects(bounds));
        assert_rect(
            frustum.project_bounds(viewport(), bounds).unwrap(),
            [50.; 4],
        );
    }
}

#[test]
fn containing_box_projects_the_frustum_even_without_visible_box_corners() {
    for projection in [
        camera().projection,
        Projection::Orthographic { vertical_size: 4. },
    ] {
        for radius in [100., 1e30] {
            let camera = Camera {
                projection,
                ..camera()
            };
            let bounds = Aabb::new([-radius; 3], [radius; 3]).unwrap();
            assert_rect(
                camera.project_bounds(viewport(), bounds).unwrap(),
                [0., 0., 100., 100.],
            );
        }
    }
    let camera = Camera {
        far: 1e30,
        ..camera()
    };
    assert_rect(
        camera
            .project_bounds(
                viewport(),
                Aabb::new([-1., -1., -4.], [1., 1., -2.]).unwrap(),
            )
            .unwrap(),
        [25., 25., 75., 75.],
    );
}

#[test]
fn projection_rejects_corner_false_positives_from_conservative_plane_tests() {
    let camera = Camera {
        up: [1., 1., 0.],
        projection: Projection::Orthographic { vertical_size: 2. },
        ..camera()
    };
    let frustum = camera.frustum(1.).unwrap();
    let bounds = Aabb::new([1.5, -2., -3.], [1.7, 2., -2.]).unwrap();
    assert!(frustum.intersects(bounds));
    assert_eq!(frustum.project_bounds(viewport(), bounds).unwrap(), None);
}

#[test]
fn lens_shift_viewport_scaling_and_snapshot_ownership_are_independent() {
    let bounds = Aabb::new([-0.5, -0.5, -3.], [0.5, 0.5, -2.]).unwrap();
    for projection in [
        camera().projection,
        Projection::Orthographic { vertical_size: 4. },
    ] {
        let mut camera = Camera {
            projection,
            lens_shift: [0.25, -0.5],
            ..camera()
        };
        let frustum = camera.frustum(1.).unwrap();
        let base = frustum.project_bounds(viewport(), bounds).unwrap().unwrap();
        assert_rect(Some(base), [25., 12.5, 50., 37.5]);
        for scale in [1., 1.5, 2.] {
            let view = Bounds::new(
                point(px(31.25 * scale), px(53.5 * scale)),
                size(px(100. * scale), px(100. * scale)),
            );
            let expected = [56.25, 66., 81.25, 91.].map(|v| v * scale);
            let actual = camera.project_bounds(view, bounds).unwrap();
            assert_rect(actual, expected);
            assert_eq!(actual, frustum.project_bounds(view, bounds).unwrap());
        }
        camera.eye[0] = 50.;
        camera.target[0] = 50.;
        assert_eq!(camera.project_bounds(viewport(), bounds).unwrap(), None);
        assert_eq!(
            frustum.project_bounds(viewport(), bounds).unwrap(),
            Some(base)
        );
    }
}

#[test]
fn pixel_extents_round_outward_and_reject_unrepresentable_endpoints() {
    let frustum = camera().frustum(1.).unwrap();
    let bounds = Aabb::new([-100.; 3], [100.; 3]).unwrap();
    for offset in [0.1_f32, 31.25, -2048.125, 1e8] {
        for width in [0.1_f32, 100.2, 555.555, 1e7] {
            let view = Bounds::new(point(px(offset), px(offset)), size(px(width), px(width)));
            let rect = coordinates(frustum.project_bounds(view, bounds).unwrap().unwrap());
            for lower in &rect[..2] {
                assert!(f64::from(*lower) <= f64::from(offset));
            }
            for upper in &rect[2..] {
                assert!(
                    f64::from(*upper) >= f64::from(offset) + f64::from(width),
                    "offset={offset} width={width} rect={rect:?}"
                );
            }
        }
    }
    let view = Bounds::new(point(px(f32::MAX), px(0.)), size(px(f32::MAX), px(100.)));
    assert_eq!(
        frustum.project_bounds(view, bounds),
        Err(CameraError::Unrepresentable)
    );
    for view in [
        Bounds::new(point(px(f32::NAN), px(0.)), size(px(100.), px(100.))),
        Bounds::new(point(px(0.), px(0.)), size(px(0.), px(100.))),
    ] {
        assert_eq!(
            frustum.project_bounds(view, bounds),
            Err(CameraError::InvalidViewport)
        );
    }
}

#[test]
fn projected_rectangles_contain_clipped_ray_hits_across_camera_poses() {
    let boxes = [
        Aabb::new([-1.; 3], [1.; 3]).unwrap(),
        Aabb::new([-6., -0.5, -6.], [6., 0.5, 6.]).unwrap(),
        Aabb::new([2., -3., -4.], [4., 3., 2.]).unwrap(),
        Aabb::new([-20.; 3], [20.; 3]).unwrap(),
    ];
    for (projection, far) in [
        (camera().projection, 20.),
        (Projection::Orthographic { vertical_size: 8. }, 20.),
        (camera().projection, f32::INFINITY),
    ] {
        for (eye, up) in [([3., 2., 6.], [0., 1., 0.]), ([-4., 3., 1.], [1., 1., 0.])] {
            for lens_shift in [[0.; 2], [0.7, -0.4]] {
                let camera = Camera {
                    aspect_ratio: None,
                    eye,
                    target: [0.; 3],
                    up,
                    projection,
                    lens_shift,
                    near: 0.5,
                    far,
                };
                let forward = camera.axes().unwrap()[2].map(|v| -f64::from(v));
                for aspect in [0.5, 1.5, 3.] {
                    let view = Bounds::new(
                        point(px(31.25), px(53.5)),
                        size(px(180. * aspect), px(180.)),
                    );
                    let frustum = camera.frustum(aspect).unwrap();
                    let mut observed_hits = 0;
                    for bounds in boxes {
                        let projected = frustum
                            .project_bounds(view, bounds)
                            .unwrap()
                            .map(coordinates);
                        for y in 0..17 {
                            for x in 0..23 {
                                let position = view.origin
                                    + point(
                                        view.size.width * (x as f32 + 0.5) / 23.,
                                        view.size.height * (y as f32 + 0.5) / 17.,
                                    );
                                let ray = camera.screen_to_ray(view, position).unwrap();
                                let origin = ray.origin().map(f64::from);
                                let direction = ray.direction().map(f64::from);
                                let depth =
                                    -f64::from(camera.world_to_view(ray.origin()).unwrap()[2]);
                                let rate: f64 =
                                    direction.into_iter().zip(forward).map(|(a, b)| a * b).sum();
                                let mut enter = (f64::from(camera.near) - depth) / rate;
                                let mut exit = (f64::from(camera.far) - depth) / rate;
                                for axis in 0..3 {
                                    let min = f64::from(bounds.min()[axis]);
                                    let max = f64::from(bounds.max()[axis]);
                                    if direction[axis] == 0. {
                                        if origin[axis] < min || origin[axis] > max {
                                            exit = f64::NEG_INFINITY;
                                        }
                                    } else {
                                        let a = (min - origin[axis]) / direction[axis];
                                        let b = (max - origin[axis]) / direction[axis];
                                        enter = enter.max(a.min(b));
                                        exit = exit.min(a.max(b));
                                    }
                                }
                                if enter < exit {
                                    observed_hits += 1;
                                    assert!(frustum.intersects(bounds));
                                    let rect = projected
                                        .expect("clipped ray hit requires projected bounds");
                                    let pixel = [f32::from(position.x), f32::from(position.y)];
                                    for axis in 0..2 {
                                        assert!(
                                            pixel[axis] >= rect[axis] - 1e-3
                                                && pixel[axis] <= rect[axis + 2] + 1e-3,
                                            "pixel={pixel:?} rect={rect:?} camera={camera:?}"
                                        );
                                    }
                                }
                            }
                        }
                    }
                    assert!(observed_hits > 100, "insufficient sampled ray coverage");
                }
            }
        }
    }
}

#[test]
fn frustum_creation_rejects_invalid_and_singular_rendered_matrices() {
    for aspect in [0., -1., f32::NAN, f32::INFINITY] {
        assert_eq!(
            camera().frustum(aspect).unwrap_err(),
            CameraError::InvalidViewport
        );
    }
    let singular = Camera {
        near: 1e-30,
        ..Camera::default()
    };
    assert_eq!(
        singular.frustum(1.).unwrap_err(),
        CameraError::Unrepresentable
    );
    let invalid = Camera {
        eye: [0.; 3],
        target: [0.; 3],
        ..camera()
    };
    assert_eq!(invalid.frustum(1.).unwrap_err(), CameraError::InvalidView);
}
