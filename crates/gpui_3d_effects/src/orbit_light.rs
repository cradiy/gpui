use std::f32::consts::TAU;

use gpui::{Rgba, rgb};
use gpui_3d::{Material, Mesh, Object, PickBehavior, Vertex};

/// A constant-width luminous orbit with a concentrated head and a fading tail.
/// Retain this value between frames to reuse its mesh. The unit-radius orbit
/// lies in the XY plane; scale and position the returned object as needed.
/// It participates in depth testing but does not illuminate neighboring objects.
#[derive(Clone)]
pub struct OrbitLight {
    mesh: Mesh,
    color: Rgba,
    tilt: [f32; 2],
}

impl Default for OrbitLight {
    fn default() -> Self {
        Self {
            mesh: mesh(),
            color: rgb(0xffdfaa),
            tilt: [0.8, 0.3],
        }
    }
}

impl OrbitLight {
    /// Sets the unlit sRGB strand color.
    pub fn color(mut self, color: impl Into<Rgba>) -> Self {
        self.color = color.into();
        self
    }

    /// Sets X/Y tilt in radians. Non-finite angles become zero.
    pub fn tilt(mut self, tilt: [f32; 2]) -> Self {
        self.tilt = tilt.map(angle);
        self
    }

    /// Samples the highlight's phase in radians along the fixed tilted orbit.
    /// The tail trails increasing phase. Geometry is shared between samples.
    /// Add the object to the same scene as its occluders. Bloom can be applied to
    /// the viewport separately; no clock or redraw loop is owned by this effect.
    pub fn object(&self, phase: f32) -> Object {
        Object::new(self.mesh.clone(), Material::color(self.color).unlit(true))
            .rotation(orbit_rotation(self.tilt, angle(phase)))
            .pick_behavior(PickBehavior::Ignore)
            .cast_shadows(false)
            .receive_shadows(false)
    }
}

fn orbit_rotation([x, y]: [f32; 2], phase: f32) -> [f32; 3] {
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sp, cp) = phase.sin_cos();
    // Ry(tilt.y) * Rx(tilt.x) * Rz(phase), expressed as XYZ Euler angles.
    let r00 = cy * cp + sy * sx * sp;
    let r10 = cx * sp;
    let r20 = -sy * cp + cy * sx * sp;
    let cos_y = r00.hypot(r10);
    if cos_y > 1e-6 {
        [
            (sy * sp + cy * sx * cp).atan2(cy * cx),
            (-r20).atan2(cos_y),
            r10.atan2(r00),
        ]
    } else {
        [sx.atan2(cx * cp), (-r20).atan2(cos_y), 0.]
    }
}

fn angle(value: f32) -> f32 {
    if value.is_finite() {
        value.rem_euclid(TAU)
    } else {
        0.
    }
}

fn mesh() -> Mesh {
    const SEGMENTS: u32 = 512;
    const SIDES: u32 = 8;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut colors = Vec::new();
    for ring in 0..SEGMENTS {
        let a = ring as f32 / SEGMENTS as f32 * TAU;
        let offset = (a + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI;
        let extent = if offset <= 0. { 2.6 } else { 0.16 };
        let t = (1. - offset.abs() / extent).clamp(0., 1.);
        let envelope = t * t * (3. - 2. * t);
        let width = 0.008;
        let brightness = 0.18 + 0.82 * envelope * envelope;
        for side in 0..SIDES {
            let b = side as f32 / SIDES as f32 * TAU;
            let radial = 1. + width * b.cos();
            vertices.push(Vertex {
                position: [a.cos() * radial, a.sin() * radial, width * b.sin()],
                normal: [a.cos() * b.cos(), a.sin() * b.cos(), b.sin()],
                uv: [ring as f32 / SEGMENTS as f32, side as f32 / SIDES as f32],
            });
            colors.push([brightness, brightness, brightness, 1.]);
            let current = ring * SIDES + side;
            let next = ((ring + 1) % SEGMENTS) * SIDES + side;
            let around = ring * SIDES + (side + 1) % SIDES;
            let diagonal = ((ring + 1) % SEGMENTS) * SIDES + (side + 1) % SIDES;
            indices.extend([current, next, diagonal, current, diagonal, around]);
        }
    }
    Mesh::new(vertices, indices)
        .with_vertex_colors(colors)
        .expect("finite orbit colors")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn orbit_is_a_closed_surface_with_outward_faces() {
        let mesh = mesh();
        let mut edges = HashMap::new();
        for triangle in mesh.indices().chunks_exact(3) {
            let [a, b, c] =
                [triangle[0], triangle[1], triangle[2]].map(|i| mesh.vertices()[i as usize]);
            let u: [f32; 3] = std::array::from_fn(|i| b.position[i] - a.position[i]);
            let v: [f32; 3] = std::array::from_fn(|i| c.position[i] - a.position[i]);
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            assert!(cross.iter().zip(a.normal).map(|(x, n)| x * n).sum::<f32>() > 0.);
            for (x, y) in [
                (triangle[0], triangle[1]),
                (triangle[1], triangle[2]),
                (triangle[2], triangle[0]),
            ] {
                let entry = edges.entry((x.min(y), x.max(y))).or_insert((0, 0));
                entry.0 += 1;
                entry.1 += if x < y { 1 } else { -1 };
            }
        }
        assert!(edges.values().all(|edge| *edge == (2, 0)));
    }
}
