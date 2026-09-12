use super::*;
use crate::{Material, Mesh, Object, Ray};

#[test]
fn ray_hits_preserve_geometry_across_mesh_scales() {
    let plane = Mesh::plane();
    for scale in [1e-20_f32, 1e-12, 1., 1e10, 1e20] {
        let mut vertices = plane.vertices().to_vec();
        for vertex in &mut vertices {
            vertex.position = vertex.position.map(|value| value * scale);
        }
        let scene = Scene::new().object(Object::new(
            Mesh::new(vertices, plane.indices().to_vec()),
            Material::color(gpui::white()),
        ));
        for side in [-1., 1.] {
            let ray = Ray::new([0.2 * scale, 0.1 * scale, side * 2.], [0., 0., -side]).unwrap();
            let hit = scene
                .raycast(ray)
                .unwrap_or_else(|| panic!("scale {scale}, side {side}"));
            assert!(
                (hit.distance - 2.).abs() < 1e-5,
                "scale {scale}: {}",
                hit.distance
            );
            assert!((hit.position[0] / scale - 0.2).abs() < 1e-5);
            assert!((hit.position[1] / scale - 0.1).abs() < 1e-5);
            assert!(hit.position[2].abs() < 1e-5);
            assert!((hit.uv[0] - 0.7).abs() < 1e-5);
            assert!((hit.uv[1] - 0.4).abs() < 1e-5);
            assert_eq!(hit.normal, [0., 0., side]);
            let outside = Ray::new([2. * scale, 0., side * 2.], [0., 0., -side]).unwrap();
            assert!(scene.raycast(outside).is_none());
        }
    }
}

#[test]
fn ray_hit_normals_remain_unit_under_normal_axis_scaling() {
    for scale in [-1e10_f32, -1., -1e-4, 1e-4, 1., 1e10] {
        let scene = Scene::new().object(
            Object::new(Mesh::plane(), Material::color(gpui::white())).scale([1., 1., scale]),
        );
        for side in [-1., 1.] {
            let ray = Ray::new([0.2, 0.1, 2. * side], [0., 0., -side]).unwrap();
            let hit = scene.raycast(ray).unwrap();
            assert_eq!(hit.normal, [0., 0., side], "scale {scale}");
        }
    }
}

#[test]
fn captured_drag_extrapolates_small_and_large_surfaces() {
    use crate::{Camera, Projection};
    use gpui::{point, px, size};

    let bounds = Bounds::new(point(px(10.), px(20.)), size(px(100.), px(100.)));
    for scale in [1e-20_f32, 1., 1e20] {
        let plane = Mesh::plane();
        let mut vertices = plane.vertices().to_vec();
        for vertex in &mut vertices {
            vertex.position = vertex.position.map(|value| value * scale);
        }
        let snapshot = PickSnapshot {
            scene: Scene::new()
                .camera(Camera {
                    eye: [0., 0., 2.],
                    projection: Projection::Orthographic {
                        vertical_size: scale * 2.,
                    },
                    ..Default::default()
                })
                .object(Object::new(
                    Mesh::new(vertices, plane.indices().to_vec()),
                    Material::color(gpui::white()),
                )),
            bounds,
            surfaces: vec![PickSurface::Solid],
        };
        let hit = snapshot.pick(point(px(60.), px(70.))).unwrap();
        let drag = DragProjection::new(&snapshot, &hit);
        let outside = point(px(135.), px(95.));
        assert!(snapshot.pick(outside).is_none());
        let uv = drag.project(outside).unwrap();
        assert!((uv[0] - 2.).abs() < 1e-5, "scale {scale}: {uv:?}");
        assert!((uv[1] - 1.).abs() < 1e-5, "scale {scale}: {uv:?}");
    }
}
