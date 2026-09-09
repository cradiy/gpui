use gpui_3d::{Material, Mesh, MeshError, Object, Ray, Scene, Vertex, VertexAttribute};

fn vertices() -> Vec<Vertex> {
    [[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]]
        .map(|position| Vertex {
            position,
            normal: [0., 0., 1.],
            uv: [0.; 2],
        })
        .to_vec()
}

fn rejects(vertices: Vec<Vertex>, indices: Vec<u32>, expected: MeshError) {
    assert_eq!(
        Mesh::try_new(vertices.clone(), indices.clone()).unwrap_err(),
        expected
    );
    assert_eq!(
        gpui::Mesh3d::try_new(vertices, indices).unwrap_err(),
        expected
    );
}

#[test]
fn malformed_meshes_report_actionable_locations() {
    rejects(vec![], vec![], MeshError::EmptyVertices);
    rejects(vertices(), vec![], MeshError::EmptyIndices);
    rejects(
        vertices(),
        vec![0, 1, 2, 0],
        MeshError::IncompleteTriangle { index_count: 4 },
    );
    for index in [3, u32::MAX] {
        rejects(
            vertices(),
            vec![0, index, 2],
            MeshError::IndexOutOfBounds {
                offset: 1,
                index,
                vertex_count: 3,
            },
        );
    }
    for (attribute, components) in [
        (VertexAttribute::Position, 3),
        (VertexAttribute::Normal, 3),
        (VertexAttribute::Uv, 2),
    ] {
        for component in 0..components {
            for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                let mut data = vertices();
                let mut unused = data[0];
                let values: &mut [f32] = match attribute {
                    VertexAttribute::Position => &mut unused.position,
                    VertexAttribute::Normal => &mut unused.normal,
                    VertexAttribute::Uv => &mut unused.uv,
                };
                values[component] = value;
                data.push(unused);
                rejects(
                    data,
                    vec![0, 1, 2],
                    MeshError::NonFiniteVertex {
                        vertex: 3,
                        attribute,
                        component,
                    },
                );
            }
        }
    }
}

#[test]
fn inspection_shares_storage_and_preserves_triangle_identity() {
    let mut data = vertices();
    data.push(Vertex {
        position: [8., 4., -2.],
        normal: [0.; 3],
        uv: [-2., 3.],
    });
    let indices = vec![0, 0, 0, 0, 1, 2];
    let mesh = Mesh::try_new(data.clone(), indices.clone()).unwrap();
    let copy = mesh.clone();
    assert!(std::ptr::eq(mesh.vertices(), copy.vertices()));
    assert!(std::ptr::eq(mesh.indices(), copy.indices()));
    assert_eq!(mesh.vertex_count(), data.len());
    assert_eq!(mesh.index_count(), indices.len());
    assert_eq!(mesh.triangle_count(), 2);
    assert_eq!(mesh.indices(), indices);
    for (actual, expected) in mesh.vertices().iter().zip(&data) {
        assert_eq!(actual.position, expected.position);
        assert_eq!(actual.normal, expected.normal);
        assert_eq!(actual.uv, expected.uv);
    }
    assert_eq!(mesh.bounds().min(), [-1., -1., -2.]);
    assert_eq!(mesh.bounds().max(), [8., 4., 0.]);
    let scene = Scene::new().object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))));
    let ray = Ray::new([0., 0., 1.], [0., 0., -1.]).unwrap();
    let hit = scene.raycast(ray).unwrap();
    assert_eq!(hit.triangle_index, 1);
    assert_eq!(hit.position, [0.; 3]);
    let degenerate = Mesh::try_new(data, vec![0, 0, 0]).unwrap();
    assert!(
        Scene::new()
            .object(Object::new(
                degenerate,
                Material::color(gpui::rgb(0xffffff))
            ))
            .raycast(ray)
            .is_none()
    );
    drop(scene);
    assert_eq!(copy.indices(), indices);
}

#[test]
fn tangent_inputs_preserve_geometry_and_reject_undefined_bases() {
    use gpui_3d::TangentError;
    let source = Mesh::new(vertices(), vec![0, 1, 2]);
    let mesh = source.with_tangents(vec![[2., 0., 1., -1.]; 3]).unwrap();
    assert!(source.tangents().is_none());
    assert_eq!(mesh.tangents().unwrap(), &[[1., 0., 0., -1.]; 3]);
    assert!(std::ptr::eq(source.vertices(), mesh.vertices()));
    assert!(std::ptr::eq(source.indices(), mesh.indices()));
    let ray = Ray::new([0., 0., 1.], [0., 0., -1.]).unwrap();
    let hit = |mesh| {
        Scene::new()
            .object(Object::new(mesh, Material::color(gpui::rgb(0xffffff))))
            .raycast(ray)
            .unwrap()
    };
    let before = hit(source.clone());
    let after = hit(mesh);
    assert_eq!(before.position, after.position);
    assert_eq!(before.normal, after.normal);
    assert_eq!(before.triangle_index, after.triangle_index);
    assert_eq!(
        source.with_tangents(vec![]).unwrap_err(),
        TangentError::Count {
            expected: 3,
            actual: 0
        }
    );
    for invalid in [
        [0.; 4],
        [0., 0., 1., 1.],
        [1., 0., 0., 0.],
        [f32::NAN, 0., 0., 1.],
    ] {
        let mut tangents = vec![[1., 0., 0., 1.]; 3];
        tangents[1] = invalid;
        assert_eq!(
            source.with_tangents(tangents).unwrap_err(),
            TangentError::InvalidBasis { vertex: 1 }
        );
    }
    assert_eq!(
        source
            .with_tangents(vec![[1., 0., 0., 1.], [1., 0., 0., -1.], [1., 0., 0., 1.]])
            .unwrap_err(),
        TangentError::MixedHandedness { triangle: 0 }
    );
    let mut data = vertices();
    data[2].normal = [0.; 3];
    assert_eq!(
        Mesh::new(data, vec![0, 1, 2])
            .with_tangents(vec![[1., 0., 0., 1.]; 3])
            .unwrap_err(),
        TangentError::InvalidBasis { vertex: 2 }
    );
}
