use gpui::{Bounds, Pixels, point, px, rgb, size};
use gpui_3d::{
    Aabb, AffineTransform, Camera, CameraError, Material, Mesh, Object, OrbitController,
    Projection, ResolvedTexture, Scene, TextureState,
};

fn viewport(aspect: f32, scale: f32) -> Bounds<Pixels> {
    Bounds::new(
        point(px(31. * scale), px(53. * scale)),
        size(px(600. * aspect * scale), px(600. * scale)),
    )
}
fn close(a: f32, b: f32) {
    assert!((a - b).abs() < 3e-4, "{a} != {b}");
}
fn camera(projection: Projection, lens_shift: [f32; 2]) -> Camera {
    Camera {
        eye: [0.; 3],
        target: [0., 0., -1.],
        projection,
        lens_shift,
        near: 0.1,
        far: 100.,
        ..Default::default()
    }
}
fn projections() -> [Projection; 2] {
    [
        Projection::from_focal_length(50., 24.).unwrap(),
        Projection::Orthographic { vertical_size: 4. },
    ]
}

#[test]
fn focal_length_conversion_matches_sensor_plane_coverage_and_rejects_invalid_optics() {
    for focal in [8., 50., 300.] {
        for sensor in [6.4, 24., 54.] {
            let projection = Projection::from_focal_length(focal, sensor).unwrap();
            assert!((projection.focal_length(sensor).unwrap() / focal - 1.).abs() < 1e-6);
            let camera = camera(projection, [0.; 2]);
            let rect = viewport(1.5, 1.);
            let edge = camera
                .world_to_screen(rect, [sensor * 0.75, sensor * 0.5, -focal])
                .unwrap()
                .unwrap();
            close(edge.ndc[0], 1.);
            close(edge.ndc[1], 1.);
            let doubled = Projection::from_focal_length(focal * 1000., sensor * 1000.).unwrap();
            close(
                camera.projection_matrix(1.5).unwrap()[1][1],
                Camera {
                    projection: doubled,
                    ..Default::default()
                }
                .projection_matrix(1.5)
                .unwrap()[1][1],
            );
        }
    }
    for invalid in [0., -1., f32::NAN, f32::INFINITY] {
        assert_eq!(
            Projection::from_focal_length(invalid, 24.),
            Err(CameraError::InvalidProjection)
        );
        assert_eq!(
            Projection::from_focal_length(50., invalid),
            Err(CameraError::InvalidProjection)
        );
        assert_eq!(
            Projection::default().focal_length(invalid),
            Err(CameraError::InvalidProjection)
        );
    }
    assert_eq!(
        Projection::Orthographic { vertical_size: 1. }.focal_length(24.),
        Err(CameraError::InvalidProjection)
    );
    assert_eq!(
        Projection::from_focal_length(f32::MAX, f32::from_bits(1)),
        Err(CameraError::Unrepresentable)
    );
    assert_eq!(
        Projection::from_focal_length(f32::from_bits(1), f32::MAX),
        Err(CameraError::Unrepresentable)
    );
}

#[test]
fn shifted_matrix_rays_and_screen_coordinates_agree_across_camera_poses() {
    for projection in projections() {
        for shift in [[0.; 2], [0.7, -0.3], [2., 1.5]] {
            let base = camera(projection, shift);
            for pose in [
                AffineTransform::IDENTITY,
                AffineTransform::from_trs([3., 2., 7.], [0.2, 0.4, 0.3, 0.8], [2., 3., 4.])
                    .unwrap(),
            ] {
                let camera = base.transformed(pose).unwrap();
                assert_eq!(camera.lens_shift, shift);
                for aspect in [0.5, 1.5, 3.] {
                    let rect = viewport(aspect, 1.5);
                    let target = camera
                        .world_to_screen(rect, camera.target)
                        .unwrap()
                        .unwrap();
                    close(target.ndc[0], -shift[0]);
                    close(target.ndc[1], -shift[1]);
                    for uv in [[0., 0.], [0.5, 0.5], [1., 1.], [-0.2, 1.2]] {
                        let pixel =
                            rect.origin + point(rect.size.width * uv[0], rect.size.height * uv[1]);
                        let ray = camera.screen_to_ray(rect, pixel).unwrap();
                        let world = ray.at(8.);
                        let screen = camera.world_to_screen(rect, world).unwrap().unwrap();
                        let reconstructed = camera
                            .screen_to_world(rect, screen.position, screen.depth)
                            .unwrap();
                        for (actual, expected) in reconstructed.into_iter().zip(world) {
                            close(actual, expected);
                        }
                        close(
                            f32::from(screen.position.x) / 600.,
                            f32::from(pixel.x) / 600.,
                        );
                        close(
                            f32::from(screen.position.y) / 600.,
                            f32::from(pixel.y) / 600.,
                        );
                        let matrix = camera.view_projection(aspect).unwrap();
                        let point = [world[0], world[1], world[2], 1.];
                        let clip: [f32; 4] =
                            std::array::from_fn(|r| (0..4).map(|c| matrix[c][r] * point[c]).sum());
                        for (i, ndc) in screen.ndc.into_iter().enumerate() {
                            close(ndc, clip[i] / clip[3]);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn linear_depth_reconstruction_preserves_offsets_scale_and_unclipped_positions() {
    for projection in projections() {
        let camera = camera(projection, [0.25, -0.5]);
        for scale in [1., 1.5, 2.] {
            let rect = viewport(1.5, scale);
            for uv in [[0.5, 0.5], [0., 1.], [-0.3, 1.2]] {
                let pixel = rect.origin + point(rect.size.width * uv[0], rect.size.height * uv[1]);
                for depth in [0.025, 0.1, 2., 100., 150.] {
                    let world = camera.screen_to_world(rect, pixel, depth).unwrap();
                    let projected = camera.world_to_screen(rect, world).unwrap().unwrap();
                    close(projected.depth, depth);
                    close(f32::from(projected.position.x - pixel.x) / scale, 0.);
                    close(f32::from(projected.position.y - pixel.y) / scale, 0.);
                    let reference = camera
                        .screen_to_world(
                            viewport(1.5, 1.),
                            point(pixel.x / scale, pixel.y / scale),
                            depth,
                        )
                        .unwrap();
                    for (actual, expected) in world.into_iter().zip(reference) {
                        close(actual, expected);
                    }
                    let ray = camera.screen_to_ray(rect, pixel).unwrap();
                    let distance = depth / -ray.direction()[2];
                    for (actual, expected) in world.into_iter().zip(ray.at(distance)) {
                        close(actual, expected);
                    }
                }
            }
        }
    }
}

#[test]
fn linear_depth_reconstruction_rejects_invalid_inputs_and_checks_result_precision() {
    let camera = camera(Projection::Orthographic { vertical_size: 4. }, [0.; 2]);
    let rect = viewport(1., 1.);
    for projection in projections() {
        for depth in [-1., f32::INFINITY, f32::NAN] {
            assert_eq!(
                Camera {
                    projection,
                    ..camera
                }
                .screen_to_world(rect, rect.center(), depth),
                Err(CameraError::InvalidDepth)
            );
        }
    }
    assert_eq!(
        camera.screen_to_world(rect, rect.center(), 0.),
        Ok(camera.eye)
    );
    assert_eq!(
        Camera {
            projection: Projection::default(),
            ..camera
        }
        .screen_to_world(rect, rect.center(), 0.),
        Err(CameraError::InvalidDepth)
    );
    assert_eq!(
        camera.screen_to_world(rect, point(px(f32::NAN), px(0.)), 1.),
        Err(CameraError::InvalidPoint)
    );
    assert_eq!(
        camera.screen_to_world(Bounds::default(), rect.center(), 1.),
        Err(CameraError::InvalidViewport)
    );
    assert_eq!(
        Camera {
            near: -1.,
            ..camera
        }
        .screen_to_world(rect, rect.center(), 1.),
        Err(CameraError::InvalidProjection)
    );
    assert_eq!(
        Camera {
            eye: camera.target,
            ..camera
        }
        .screen_to_world(rect, rect.center(), 1.),
        Err(CameraError::InvalidView)
    );
    let wide = Bounds::new(
        point(px(-f32::MAX * 0.75), px(0.)),
        size(px(f32::MAX), px(f32::MAX)),
    );
    let world = camera
        .screen_to_world(wide, point(px(f32::MAX * 0.75), px(0.)), 1.)
        .unwrap();
    close(world[0], 4.);
    close(world[1], 2.);
    close(world[2], -1.);
    assert_eq!(
        Camera {
            projection: Projection::default(),
            ..camera
        }
        .screen_to_world(rect, point(px(f32::MAX), px(0.)), f32::MAX),
        Err(CameraError::Unrepresentable)
    );
}

#[test]
fn shifted_culling_and_picking_use_the_visible_region_instead_of_the_optical_axis() {
    for projection in projections() {
        let camera = camera(projection, [2., 0.]);
        let rect = viewport(1., 1.);
        let ray = camera.screen_to_ray(rect, rect.center()).unwrap();
        let distance = (-5. - ray.origin()[2]) / ray.direction()[2];
        let center = ray.at(distance);
        let scene = Scene::new()
            .camera(camera)
            .object(
                Object::new(Mesh::plane(), Material::image("outside.png")).position([0., 0., -5.]),
            )
            .object(
                Object::new(Mesh::plane(), Material::color(rgb(0xffffff)))
                    .position(center)
                    .id("shifted"),
            );
        let mut requests = Vec::new();
        let prepared = scene
            .prepare(1., None, |request| {
                requests.push(request.object_index);
                Ok(TextureState::Ready(ResolvedTexture::None))
            })
            .unwrap();
        assert_eq!(requests, [1]);
        assert_eq!(prepared.frame().objects[0].output_id, 2);
        assert_eq!(
            scene.pick(rect, rect.center()).unwrap().object_id,
            Some("shifted".into())
        );
    }
}

#[test]
fn shifted_framing_centers_bounds_and_preserves_margins_and_optics() {
    let bounds = Aabb::new([-3., -1., -2.], [4., 2., 1.]).unwrap();
    let center = [0.5, 0.5, -0.5];
    for projection in projections() {
        for lens_shift in [[0.7, -0.4], [-2., 1.5]] {
            for aspect in [0.25, 1., 3.] {
                let camera = Camera {
                    projection,
                    lens_shift,
                    up: [0.3, 1., 0.2],
                    ..Camera::orbit(0.5, 0.3, 8.)
                };
                let framed = camera.frame_bounds(bounds, aspect, 1.3).unwrap();
                assert_eq!(framed.lens_shift, lens_shift);
                for i in 0..3 {
                    close(camera.axes().unwrap()[2][i], framed.axes().unwrap()[2][i]);
                }
                let middle = framed
                    .world_to_screen(viewport(aspect, 1.), center)
                    .unwrap()
                    .unwrap();
                close(middle.ndc[0], 0.);
                close(middle.ndc[1], 0.);
                for corner in 0..8 {
                    let p = std::array::from_fn(|i| {
                        if corner & (1 << i) == 0 {
                            bounds.min()[i]
                        } else {
                            bounds.max()[i]
                        }
                    });
                    let screen = framed
                        .world_to_screen(viewport(aspect, 1.), p)
                        .unwrap()
                        .unwrap();
                    assert!(screen.in_frustum);
                    assert!(
                        screen.ndc[0].abs() <= 1. / 1.3 + 1e-4
                            && screen.ndc[1].abs() <= 1. / 1.3 + 1e-4
                    );
                }
            }
        }
    }
    for value in [f32::NAN, f32::INFINITY] {
        let invalid = camera(Projection::default(), [value, 0.]);
        assert_eq!(
            invalid.projection_matrix(1.),
            Err(CameraError::InvalidProjection)
        );
        assert!(
            invalid
                .screen_to_ray(viewport(1., 1.), point(px(0.), px(0.)))
                .is_err()
        );
        assert!(OrbitController::new(invalid).is_err());
    }
}

#[test]
fn orbit_pan_dolly_and_optical_zoom_retain_the_shifted_principal_point() {
    for projection in projections() {
        let camera = Camera {
            projection,
            lens_shift: [0.8, -0.3],
            ..Camera::orbit(0.4, 0.2, 7.)
        };
        let mut controls = OrbitController::new(camera).unwrap();
        let rect = viewport(1.5, 1.);
        let before = camera
            .world_to_screen(rect, camera.target)
            .unwrap()
            .unwrap();
        controls.pan_by(rect, point(px(40.), px(-25.))).unwrap();
        let after = controls
            .camera()
            .world_to_screen(rect, camera.target)
            .unwrap()
            .unwrap();
        close(f32::from(after.position.x - before.position.x), 40.);
        close(f32::from(after.position.y - before.position.y), -25.);
        for action in 0..3 {
            match action {
                0 => {
                    controls.zoom(0.6).unwrap();
                }
                1 => {
                    controls.dolly(1.4).unwrap();
                }
                _ => {
                    controls.orbit_by([12., -8.]).unwrap();
                }
            }
            let changed = controls.camera();
            assert_eq!(changed.lens_shift, camera.lens_shift);
            let principal = changed
                .world_to_screen(rect, changed.target)
                .unwrap()
                .unwrap();
            close(principal.ndc[0], -camera.lens_shift[0]);
            close(principal.ndc[1], -camera.lens_shift[1]);
        }
    }
}

#[test]
fn fixed_aspect_preserves_projection_queries_and_pointer_pan_across_output_sizes() {
    for projection in projections() {
        let camera = Camera {
            aspect_ratio: Some(2.),
            ..camera(projection, [0.3, -0.2])
        };
        let matrix = camera.projection_matrix(1.).unwrap();
        assert_eq!(matrix, camera.projection_matrix(3.).unwrap());
        let world = [1., 0.5, -4.];
        let mut previous_ndc = None;
        for aspect in [0.5, 1., 3.] {
            let rect = viewport(aspect, 1.5);
            let projected = camera.world_to_screen(rect, world).unwrap().unwrap();
            if let Some(ndc) = previous_ndc {
                assert_eq!(projected.ndc, ndc);
            }
            previous_ndc = Some(projected.ndc);
            let restored = camera
                .screen_to_world(rect, projected.position, projected.depth)
                .unwrap();
            for i in 0..3 {
                close(restored[i], world[i]);
            }
            let ray = camera.screen_to_ray(rect, projected.position).unwrap();
            let distance = (world[2] - ray.origin()[2]) / ray.direction()[2];
            for (actual, expected) in ray.at(distance).into_iter().zip(world) {
                close(actual, expected);
            }
            let mut orbit = OrbitController::new(camera).unwrap();
            let before = camera
                .world_to_screen(rect, camera.target)
                .unwrap()
                .unwrap()
                .position;
            orbit.pan_by(rect, point(px(20.), px(-10.))).unwrap();
            let after = orbit
                .camera()
                .world_to_screen(rect, camera.target)
                .unwrap()
                .unwrap()
                .position;
            close(f32::from(after.x - before.x), 20.);
            close(f32::from(after.y - before.y), -10.);
            assert_eq!(orbit.camera().aspect_ratio, Some(2.));
        }
    }
}

#[test]
fn fixed_aspect_framing_contains_all_bounds_in_both_projection_modes() {
    let bounds = Aabb::new([-5., -1., -2.], [3., 2., 1.]).unwrap();
    for projection in projections() {
        for aspect in [0.25, 4.] {
            let camera = Camera {
                aspect_ratio: Some(aspect),
                ..camera(projection, [0.4, 0.1])
            };
            let framed = camera.frame_bounds(bounds, 1., 1.2).unwrap();
            assert_eq!(framed.aspect_ratio, Some(aspect));
            for corner in 0..8 {
                let p = std::array::from_fn(|i| {
                    if corner & (1 << i) == 0 {
                        bounds.min()[i]
                    } else {
                        bounds.max()[i]
                    }
                });
                let hit = framed
                    .world_to_screen(viewport(1., 1.), p)
                    .unwrap()
                    .unwrap();
                assert!(hit.in_frustum, "{hit:?}");
                assert!(hit.ndc[0].abs() <= 1. / 1.19 && hit.ndc[1].abs() <= 1. / 1.19);
            }
        }
    }
}

#[test]
fn infinite_perspective_has_finite_matrices_and_unbounded_ray_and_frustum_depth() {
    let camera = Camera {
        far: f32::INFINITY,
        aspect_ratio: Some(1.),
        ..camera(
            Projection::Perspective {
                vertical_fov: std::f32::consts::FRAC_PI_2,
            },
            [0.; 2],
        )
    };
    let matrix = camera.projection_matrix(2.).unwrap();
    assert!(matrix.iter().flatten().all(|value| value.is_finite()));
    let rect = viewport(2., 1.);
    for depth in [0.1, 1., 100., 1e6] {
        let p = [depth * 0.2, 0., -depth];
        let projected = camera.world_to_screen(rect, p).unwrap().unwrap();
        close(projected.ndc[2], 1. - camera.near / depth);
        assert!(projected.in_frustum);
        let restored = camera
            .screen_to_world(rect, projected.position, depth)
            .unwrap();
        assert!((restored[0] - p[0]).abs() <= depth * 1e-5);
    }
    let bounds = Aabb::new([-10., -10., -10010.], [10., 10., -10000.]).unwrap();
    let frustum = camera.frustum(2.).unwrap();
    assert!(frustum.intersects(bounds));
    let projected = frustum.project_bounds(rect, bounds).unwrap().unwrap();
    assert!(projected.contains(&rect.center()));
    let behind = Aabb::new([-1., -1., 1.], [1., 1., 2.]).unwrap();
    assert!(!frustum.intersects(behind));
    assert!(
        Camera {
            far: 100.,
            ..camera
        }
        .project_bounds(rect, bounds)
        .unwrap()
        .is_none()
    );
    let scene = Scene::new().camera(camera).object(
        Object::new(Mesh::plane(), Material::color(rgb(0xffffff))).transform(gpui_3d::Transform {
            position: [0., 0., -10000.],
            ..Default::default()
        }),
    );
    assert!(scene.pick(rect, rect.center()).is_some());
    let prepared = scene
        .prepare(2., None, |_| Ok(TextureState::Ready(ResolvedTexture::None)))
        .unwrap();
    assert_eq!(prepared.frame().objects.len(), 1);
    assert_eq!(
        prepared.frame().view_projection,
        camera.view_projection(2.).unwrap()
    );
}

#[test]
fn invalid_fixed_aspects_and_nonperspective_infinite_depth_are_rejected() {
    for aspect in [0., -1., f32::INFINITY, f32::NAN] {
        let camera = Camera {
            aspect_ratio: Some(aspect),
            ..Default::default()
        };
        assert!(camera.projection_matrix(1.).is_err());
        assert!(OrbitController::new(camera).is_err());
    }
    let camera = Camera {
        far: f32::INFINITY,
        ..Default::default()
    };
    assert!(camera.projection_matrix(0.).is_err());
    for far in [f32::NAN, f32::NEG_INFINITY, 0.] {
        assert!(Camera { far, ..camera }.projection_matrix(1.).is_err());
    }
    assert!(
        Camera {
            projection: Projection::Orthographic { vertical_size: 2. },
            ..camera
        }
        .projection_matrix(1.)
        .is_err()
    );
}
