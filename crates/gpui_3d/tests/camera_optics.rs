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
