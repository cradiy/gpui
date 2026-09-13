use std::f64::consts::TAU;

use anyhow::{Result, ensure};
use gpui_3d::{Mesh, Vertex};

type Vector = [f64; 3];
const SIDES: usize = 8;

fn add(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub(a: Vector, b: Vector) -> Vector {
    std::array::from_fn(|i| a[i] - b[i])
}
fn mul(a: Vector, scale: f64) -> Vector {
    a.map(|v| v * scale)
}
fn dot(a: Vector, b: Vector) -> f64 {
    a.into_iter().zip(b).map(|(x, y)| x * y).sum()
}
fn cross(a: Vector, b: Vector) -> Vector {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: Vector) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: Vector) -> Vector {
    mul(a, 1. / length(a))
}
fn mix(a: Vector, b: Vector, t: f64) -> Vector {
    add(mul(a, 1. - t), mul(b, t))
}

pub(super) fn smooth(points: impl IntoIterator<Item = [f32; 3]>) -> Result<Vec<[f32; 3]>> {
    let points: Vec<_> = points.into_iter().take(4097).collect();
    ensure!(
        points.len() <= 4096 && points.iter().flatten().all(|v| v.is_finite()),
        "curve light requires at most 4096 finite path points"
    );
    let mut controls: Vec<Vector> = Vec::new();
    for point in points {
        let point = point.map(f64::from);
        if controls
            .last()
            .is_none_or(|last| length(sub(point, *last)) > 1e-8)
        {
            controls.push(point);
        }
    }
    ensure!(
        controls.len() >= 2,
        "curve light requires two distinct path points"
    );
    ensure!(
        length(sub(controls[0], controls[controls.len() - 1])) > 1e-8,
        "curve light requires an open path"
    );
    let mut samples = Vec::new();
    for i in 0..controls.len() - 1 {
        let p1 = controls[i];
        let p2 = controls[i + 1];
        let p0 = if i == 0 {
            sub(mul(p1, 2.), p2)
        } else {
            controls[i - 1]
        };
        let p3 = controls
            .get(i + 2)
            .copied()
            .unwrap_or_else(|| sub(mul(p2, 2.), p1));
        let t1 = length(sub(p1, p0)).sqrt();
        let t2 = t1 + length(sub(p2, p1)).sqrt();
        let t3 = t2 + length(sub(p3, p2)).sqrt();
        for step in 0..24 {
            let t = t1 + (t2 - t1) * step as f64 / 24.;
            let a1 = mix(p0, p1, t / t1);
            let a2 = mix(p1, p2, (t - t1) / (t2 - t1));
            let a3 = mix(p2, p3, (t - t2) / (t3 - t2));
            let b1 = mix(a1, a2, t / t2);
            let b2 = mix(a2, a3, (t - t1) / (t3 - t1));
            let point = mix(b1, b2, (t - t1) / (t2 - t1)).map(|v| v as f32);
            ensure!(
                point.iter().all(|v| v.is_finite()),
                "curve light path exceeds f32 range"
            );
            if samples.last() != Some(&point) {
                samples.push(point);
            }
        }
    }
    let end = controls[controls.len() - 1].map(|v| v as f32);
    if samples.last() != Some(&end) {
        samples.push(end);
    }
    Ok(samples)
}

pub(super) fn tube(path: &[[f32; 3]], diameter: f32) -> Result<Mesh> {
    ensure!(
        diameter.is_finite() && diameter > 0.,
        "curve light width must be positive and finite"
    );
    let path: Vec<_> = path.iter().map(|p| p.map(f64::from)).collect();
    let mut distances = vec![0.];
    for pair in path.windows(2) {
        distances.push(distances.last().unwrap() + length(sub(pair[1], pair[0])));
    }
    let total = distances[path.len() - 1];
    let mut tangents = Vec::with_capacity(path.len());
    for i in 0..path.len() {
        let mut tangent = sub(path[(i + 1).min(path.len() - 1)], path[i.saturating_sub(1)]);
        if length(tangent) < 1e-12 {
            tangent = sub(path[(i + 1).min(path.len() - 1)], path[i]);
        }
        ensure!(
            length(tangent) > 0.,
            "curve light path has an undefined tangent"
        );
        tangents.push(unit(tangent));
    }
    let first = tangents[0];
    let axis = if first[0].abs() < 0.8 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    let mut normal = unit(cross(first, axis));
    let radius = f64::from(diameter) * 0.5;
    let mut vertices = Vec::with_capacity(path.len() * SIDES + 2 * (SIDES + 1));
    let mut indices = Vec::new();
    for (i, (&point, &tangent)) in path.iter().zip(&tangents).enumerate() {
        if i > 0 {
            let previous = tangents[i - 1];
            let axis = cross(previous, tangent);
            let cosine = dot(previous, tangent).clamp(-1., 1.);
            if cosine > -0.999999 {
                normal = add(
                    add(normal, cross(axis, normal)),
                    mul(cross(axis, cross(axis, normal)), 1. / (1. + cosine)),
                );
            }
            normal = unit(sub(normal, mul(tangent, dot(normal, tangent))));
        }
        let binormal = cross(tangent, normal);
        for side in 0..SIDES {
            let angle = TAU * side as f64 / SIDES as f64;
            let radial = add(mul(normal, angle.cos()), mul(binormal, angle.sin()));
            vertices.push(Vertex {
                position: add(point, mul(radial, radius)).map(|v| v as f32),
                normal: radial.map(|v| v as f32),
                uv: [(distances[i] / total) as f32, side as f32 / SIDES as f32],
            });
            if i + 1 < path.len() {
                let a = (i * SIDES + side) as u32;
                let b = (i * SIDES + (side + 1) % SIDES) as u32;
                let c = a + SIDES as u32;
                let d = b + SIDES as u32;
                indices.extend([a, b, c, b, d, c]);
            }
        }
    }
    for end in [0, path.len() - 1] {
        let normal = mul(tangents[end], if end == 0 { -1. } else { 1. });
        let center = vertices.len() as u32;
        let uv = [if end == 0 { 0. } else { 1. }, 0.5];
        vertices.push(Vertex {
            position: path[end].map(|v| v as f32),
            normal: normal.map(|v| v as f32),
            uv,
        });
        for side in 0..SIDES {
            let position = vertices[end * SIDES + side].position;
            vertices.push(Vertex {
                position,
                normal: normal.map(|v| v as f32),
                uv,
            });
        }
        for side in 0..SIDES {
            let a = center + 1 + side as u32;
            let b = center + 1 + ((side + 1) % SIDES) as u32;
            indices.extend(if end == 0 {
                [center, b, a]
            } else {
                [center, a, b]
            });
        }
    }
    ensure!(
        vertices
            .iter()
            .all(|v| v.position.iter().chain(&v.normal).all(|v| v.is_finite())),
        "curve light geometry exceeds f32 range"
    );
    Ok(Mesh::try_new(vertices, indices)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spline_tube_keeps_width_arc_coordinates_and_outward_faces() -> Result<()> {
        let controls = [
            [-1., 0., 0.],
            [-0.8, 0.4, 0.2],
            [0.4, -0.2, -0.4],
            [1., 0.3, 0.],
        ];
        let path = smooth(controls)?;
        assert_eq!(path[0], controls[0]);
        assert_eq!(*path.last().unwrap(), controls[3]);
        let mesh = tube(&path, 0.02)?;
        let mut arc = 0.;
        let total: f64 = path
            .windows(2)
            .map(|p| length(sub(p[1].map(f64::from), p[0].map(f64::from))))
            .sum();
        for (i, point) in path.iter().enumerate() {
            if i > 0 {
                arc += length(sub(point.map(f64::from), path[i - 1].map(f64::from)));
            }
            for vertex in &mesh.vertices()[i * SIDES..(i + 1) * SIDES] {
                assert!(
                    (length(sub(vertex.position.map(f64::from), point.map(f64::from))) - 0.01)
                        .abs()
                        < 1e-6
                );
                assert!((f64::from(vertex.uv[0]) - arc / total).abs() < 1e-6);
                assert!((length(vertex.normal.map(f64::from)) - 1.).abs() < 1e-6);
            }
        }
        for face in mesh.indices().chunks_exact(3) {
            let [a, b, c] = [face[0], face[1], face[2]].map(|i| mesh.vertices()[i as usize]);
            let cross = cross(
                sub(b.position.map(f64::from), a.position.map(f64::from)),
                sub(c.position.map(f64::from), a.position.map(f64::from)),
            );
            assert!(dot(cross, a.normal.map(f64::from)) > 0.);
        }
        assert!(smooth([[0.; 3]; 3]).is_err());
        assert!(smooth([[f32::NAN; 3], [1.; 3]]).is_err());
        assert!(tube(&path, 0.).is_err());
        Ok(())
    }
}
