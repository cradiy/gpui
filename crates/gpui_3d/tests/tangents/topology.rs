use super::*;

fn corner_tangents(mesh: &Mesh) -> Vec<[f32; 4]> {
    let generated = mesh.generate_tangents().unwrap();
    generated
        .mesh()
        .indices()
        .iter()
        .map(|&index| generated.mesh().tangents().unwrap()[index as usize])
        .collect()
}

#[test]
fn signed_zero_normal_seams_keep_independent_tangent_frames() {
    let mut vertices = [
        ([0., 0., 0.], [0., 0.]),
        ([1., 0., 0.], [1., 1.]),
        ([0., 1., 0.], [0., 1.]),
        ([1., 0., 0.], [1., 1.]),
        ([0., 0., 0.], [0., 0.]),
        ([1., -1., 0.], [0., -1.]),
    ]
    .map(|(position, uv)| Vertex {
        position,
        normal: [0., 0., 1.],
        uv,
    })
    .to_vec();
    let continuous = corner_tangents(&Mesh::new(vertices.clone(), (0..6).collect()));
    near(continuous[0], continuous[4]);
    near(continuous[1], continuous[3]);
    for vertex in &mut vertices[3..] {
        vertex.normal[0] = -0.;
    }
    let isolated: Vec<_> = [vec![0, 1, 2], vec![3, 4, 5]]
        .into_iter()
        .flat_map(|indices| corner_tangents(&Mesh::new(vertices.clone(), indices)))
        .collect();
    assert!((continuous[0][0] - isolated[0][0]).abs() > 0.01);
    let seamed = corner_tangents(&Mesh::new(vertices, (0..6).collect()));
    for (actual, expected) in seamed.into_iter().zip(isolated) {
        near(actual, expected);
    }
}

#[test]
fn nonmanifold_edges_pair_opposite_directions_in_face_order() {
    let vertices = [
        ([-1., 0., 0.], [-1., 0.]),
        ([-1., 1., 0.], [-1., 0.5]),
        ([1., 0., 0.], [1., 0.]),
        ([1., 1., 0.], [1., 0.5]),
        ([0., 0., 0.], [0., 0.]),
        ([0., 1., 0.], [1., 1.]),
    ]
    .map(|(position, uv)| Vertex {
        position,
        normal: [0., 0., 1.],
        uv,
    })
    .to_vec();
    let prefix = [[0, 1, 2], [0, 2, 3]];
    let faces = [[4, 5, 0], [4, 5, 1], [5, 4, 2], [5, 4, 3]];
    let mesh = Mesh::new(
        vertices.clone(),
        prefix.into_iter().chain(faces).flatten().collect(),
    );
    let actual = corner_tangents(&mesh);
    for pair in [[0, 2], [1, 3]] {
        let reference = corner_tangents(&Mesh::new(
            vertices.clone(),
            pair.into_iter().flat_map(|face| faces[face]).collect(),
        ));
        for (side, face) in pair.into_iter().enumerate() {
            for corner in 0..3 {
                near(
                    actual[(face + 2) * 3 + corner],
                    reference[side * 3 + corner],
                );
            }
        }
    }
}
