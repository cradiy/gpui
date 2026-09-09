use std::collections::HashMap;

use gpui_3d::{
    ConeOptions, CylinderOptions, Material, Mesh, Object, PlaneOptions, PrimitiveError, Ray, Scene,
    SphereOptions,
};

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn difference(a: [f32; 3], b: [f32; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f64::from(a[i]) - f64::from(b[i]))
}

fn inspect(mesh: &Mesh, boundary_edges: usize) {
    let tangents = mesh.tangents().unwrap();
    let mut used = vec![false; mesh.vertex_count()];
    let mut edges = HashMap::new();
    let key = |index: u32| {
        mesh.vertices()[index as usize]
            .position
            .map(|v| if v == 0. { 0 } else { v.to_bits() })
    };
    for indices in mesh.indices().chunks_exact(3) {
        let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| mesh.vertices()[indices[i] as usize]);
        let ab = difference(b.position, a.position);
        let ac = difference(c.position, a.position);
        let face = cross(ab, ac);
        assert!(dot(face, face) > 0.);
        let du1 = f64::from(b.uv[0]) - f64::from(a.uv[0]);
        let dv1 = f64::from(b.uv[1]) - f64::from(a.uv[1]);
        let du2 = f64::from(c.uv[0]) - f64::from(a.uv[0]);
        let dv2 = f64::from(c.uv[1]) - f64::from(a.uv[1]);
        let det = du1 * dv2 - du2 * dv1;
        assert_ne!(det, 0.);
        let dpdu = std::array::from_fn(|i| (ab[i] * dv2 - ac[i] * dv1) / det);
        let dpdv = std::array::from_fn(|i| (ac[i] * du1 - ab[i] * du2) / det);
        for &index in indices {
            used[index as usize] = true;
            let vertex = mesh.vertices()[index as usize];
            let n = vertex.normal.map(f64::from);
            let t = tangents[index as usize];
            let tangent = [t[0], t[1], t[2]].map(f64::from);
            assert!(dot(face, n) > 0., "inward triangle at {index}");
            assert!((dot(n, n) - 1.).abs() < 1e-6);
            assert!((dot(tangent, tangent) - 1.).abs() < 1e-6);
            assert!(dot(n, tangent).abs() < 1e-6);
            assert!(dot(dpdu, tangent) > 0., "reversed U tangent at {index}");
            assert!(
                dot(dpdv, cross(n, tangent).map(|v| v * f64::from(t[3]))) > 0.,
                "reversed V tangent at {index}"
            );
            assert!(vertex.uv.iter().all(|v| (0. ..=1.).contains(v)));
        }
        for (a, b) in [
            (indices[0], indices[1]),
            (indices[1], indices[2]),
            (indices[2], indices[0]),
        ] {
            let (a, b) = (key(a), key(b));
            let (key, direction) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
            let entry = edges.entry(key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += direction;
        }
    }
    assert!(used.into_iter().all(|v| v));
    assert_eq!(
        edges.values().filter(|&&(count, _)| count == 1).count(),
        boundary_edges
    );
    for (count, balance) in edges.into_values() {
        assert!(
            count == 1 || count == 2 && balance == 0,
            "nonmanifold or inconsistent edge"
        );
    }
}

#[test]
fn primitive_topology_winding_and_tangent_frames_agree_with_uvs() {
    for [radial, vertical] in [[3, 2], [7, 3], [32, 16]] {
        let sphere = Mesh::sphere(SphereOptions {
            radius: 0.7,
            segments: [radial, vertical],
        })
        .unwrap();
        assert_eq!(
            sphere.triangle_count(),
            (2 * radial * (vertical - 1)) as usize
        );
        inspect(&sphere, 0);
        for rows in [1, vertical] {
            for capped in [false, true] {
                let cylinder = Mesh::cylinder(CylinderOptions {
                    radius: 1.3,
                    height: 2.7,
                    segments: [radial, rows],
                    capped,
                })
                .unwrap();
                assert_eq!(
                    cylinder.triangle_count(),
                    (2 * radial * (rows + u32::from(capped))) as usize
                );
                inspect(&cylinder, if capped { 0 } else { radial as usize * 2 });
                let cone = Mesh::cone(ConeOptions {
                    radius: 1.3,
                    height: 2.7,
                    segments: [radial, rows],
                    capped,
                })
                .unwrap();
                assert_eq!(
                    cone.triangle_count(),
                    (radial * (2 * rows - 1 + u32::from(capped))) as usize
                );
                inspect(&cone, if capped { 0 } else { radial as usize });
            }
        }
    }
    for segments in [[1, 1], [3, 7], [32, 16]] {
        let plane = Mesh::subdivided_plane(PlaneOptions {
            size: [2., 3.],
            segments,
        })
        .unwrap();
        assert_eq!(
            plane.triangle_count(),
            (2 * segments[0] * segments[1]) as usize
        );
        inspect(&plane, (segments[0] + segments[1]) as usize * 2);
    }
}

#[test]
fn primitive_bounds_seams_and_caps_preserve_surface_queries() {
    let sphere = Mesh::sphere(Default::default()).unwrap();
    let cylinder = Mesh::cylinder(Default::default()).unwrap();
    let cone = Mesh::cone(Default::default()).unwrap();
    for mesh in [&sphere, &cylinder, &cone] {
        assert_eq!(mesh.bounds().min(), [-0.5; 3]);
        assert_eq!(mesh.bounds().max(), [0.5; 3]);
        let copy = mesh.clone();
        assert!(std::ptr::eq(mesh.vertices(), copy.vertices()));
        assert!(std::ptr::eq(
            mesh.tangents().unwrap(),
            copy.tangents().unwrap()
        ));
    }
    for (mesh, expected_x) in [(sphere, 0.5), (cylinder, 0.5), (cone, 0.25)] {
        let scene = Scene::new().object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))));
        let hit = scene
            .raycast(Ray::new([2., 0., 0.], [-1., 0., 0.]).unwrap())
            .unwrap();
        assert!((hit.position[0] - expected_x).abs() < 1e-5);
        assert!(hit.normal[0] > 0.);
    }
    for capped in [false, true] {
        let mesh = Mesh::cylinder(CylinderOptions {
            capped,
            ..Default::default()
        })
        .unwrap();
        let scene = Scene::new().object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))));
        let hit = scene.raycast(Ray::new([0., 2., 0.], [0., -1., 0.]).unwrap());
        assert_eq!(hit.is_some(), capped);
        if let Some(hit) = hit {
            assert_eq!(hit.normal, [0., 1., 0.]);
        }
    }
    let mesh = Mesh::sphere(SphereOptions {
        segments: [7, 3],
        ..Default::default()
    })
    .unwrap();
    for row in 0..2 {
        let a = &mesh.vertices()[row * 8];
        let b = &mesh.vertices()[row * 8 + 7];
        assert_eq!(a.position, b.position);
        assert_eq!(a.normal, b.normal);
        assert_eq!((a.uv[0], b.uv[0]), (0., 1.));
        assert_eq!(
            mesh.tangents().unwrap()[row * 8],
            mesh.tangents().unwrap()[row * 8 + 7]
        );
    }
    let plane = Mesh::subdivided_plane(PlaneOptions {
        size: [4., 2.],
        segments: [3, 2],
    })
    .unwrap();
    assert_eq!(plane.bounds().min(), [-2., -1., 0.]);
    assert_eq!(plane.bounds().max(), [2., 1., 0.]);
    let scene = Scene::new().object(Object::new(plane, Material::color(gpui::rgb(0xffffff))));
    let hit = scene
        .raycast(Ray::new([1., 0.5, 2.], [0., 0., -1.]).unwrap())
        .unwrap();
    assert!((hit.uv[0] - 0.75).abs() < 1e-6 && (hit.uv[1] - 0.25).abs() < 1e-6);
}

#[test]
fn primitive_parameters_reject_invalid_or_collapsed_geometry_without_large_allocations() {
    for radius in [0., -1., f32::NAN, f32::INFINITY] {
        assert!(matches!(
            Mesh::sphere(SphereOptions {
                radius,
                ..Default::default()
            }),
            Err(PrimitiveError::Dimension {
                parameter: "radius"
            })
        ));
        assert!(
            Mesh::cylinder(CylinderOptions {
                radius,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            Mesh::cone(ConeOptions {
                height: radius,
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            Mesh::subdivided_plane(PlaneOptions {
                size: [1., radius],
                ..Default::default()
            })
            .is_err()
        );
    }
    assert!(matches!(
        Mesh::sphere(SphereOptions {
            segments: [2, 2],
            ..Default::default()
        }),
        Err(PrimitiveError::Segments {
            axis: "longitude",
            minimum: 3,
            actual: 2
        })
    ));
    assert!(
        Mesh::sphere(SphereOptions {
            segments: [3, 1],
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Mesh::cone(ConeOptions {
            segments: [3, 0],
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        Mesh::subdivided_plane(PlaneOptions {
            segments: [0, 1],
            ..Default::default()
        })
        .is_err()
    );
    for segments in [[u32::MAX; 2], [2048; 2]] {
        assert_eq!(
            Mesh::sphere(SphereOptions {
                segments,
                ..Default::default()
            })
            .unwrap_err(),
            PrimitiveError::TooLarge
        );
        assert_eq!(
            Mesh::cone(ConeOptions {
                segments,
                ..Default::default()
            })
            .unwrap_err(),
            PrimitiveError::TooLarge
        );
        assert_eq!(
            Mesh::cylinder(CylinderOptions {
                segments,
                ..Default::default()
            })
            .unwrap_err(),
            PrimitiveError::TooLarge
        );
        assert_eq!(
            Mesh::subdivided_plane(PlaneOptions {
                segments,
                ..Default::default()
            })
            .unwrap_err(),
            PrimitiveError::TooLarge
        );
    }
    assert!(matches!(
        Mesh::subdivided_plane(PlaneOptions {
            size: [f32::from_bits(1), 1.],
            ..Default::default()
        }),
        Err(PrimitiveError::Degenerate { .. })
    ));
    for radius in [1e-25, 1e25] {
        let mesh = Mesh::sphere(SphereOptions {
            radius,
            segments: [8, 4],
        })
        .unwrap();
        assert!(
            mesh.vertices()
                .iter()
                .all(|v| v.position.iter().all(|p| p.is_finite()))
        );
        assert!(mesh.bounds().max()[0] > 0.);
    }
}
