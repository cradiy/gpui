use std::{error::Error, fmt};

use crate::{Mesh, Vertex};

/// XY plane facing +Z. UV (0, 0) is the upper-left corner.
#[derive(Clone, Copy, Debug)]
pub struct PlaneOptions {
    pub size: [f32; 2],
    /// Positive horizontal and vertical subdivision counts.
    pub segments: [u32; 2],
}
impl Default for PlaneOptions {
    fn default() -> Self {
        Self {
            size: [1.; 2],
            segments: [1; 2],
        }
    }
}

/// Y-up UV sphere centered at the origin. U starts at +X toward +Z;
/// V runs from the north pole to the south pole.
#[derive(Clone, Copy, Debug)]
pub struct SphereOptions {
    pub radius: f32,
    /// Longitude sectors (at least 3) and latitude intervals (at least 2).
    pub segments: [u32; 2],
}
impl Default for SphereOptions {
    fn default() -> Self {
        Self {
            radius: 0.5,
            segments: [32, 16],
        }
    }
}

/// Y-axis cylinder centered at the origin, with separate side and cap vertices.
#[derive(Clone, Copy, Debug)]
pub struct CylinderOptions {
    pub radius: f32,
    pub height: f32,
    /// Radial sectors (at least 3) and height intervals (at least 1).
    pub segments: [u32; 2],
    pub capped: bool,
}
impl Default for CylinderOptions {
    fn default() -> Self {
        Self {
            radius: 0.5,
            height: 1.,
            segments: [32, 1],
            capped: true,
        }
    }
}

/// Y-axis cone with its tip at +height/2 and base at -height/2.
#[derive(Clone, Copy, Debug)]
pub struct ConeOptions {
    pub radius: f32,
    pub height: f32,
    /// Radial sectors (at least 3) and height intervals (at least 1).
    pub segments: [u32; 2],
    pub capped: bool,
}
impl Default for ConeOptions {
    fn default() -> Self {
        Self {
            radius: 0.5,
            height: 1.,
            segments: [32, 1],
            capped: true,
        }
    }
}

/// Invalid primitive parameters or an unrepresentable tessellation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrimitiveError {
    /// Dimensions must be finite and positive.
    Dimension { parameter: &'static str },
    Segments {
        axis: &'static str,
        minimum: u32,
        actual: u32,
    },
    /// Generation is bounded to 1,048,576 vertices and 6,291,456 indices.
    TooLarge,
    /// Dimensions and tessellation collapse a triangle in f32 mesh coordinates.
    Degenerate { triangle: usize },
}
impl fmt::Display for PrimitiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dimension { parameter } => {
                write!(f, "primitive {parameter} must be finite and positive")
            }
            Self::Segments {
                axis,
                minimum,
                actual,
            } => write!(
                f,
                "primitive {axis} segments must be at least {minimum}, got {actual}"
            ),
            Self::TooLarge => {
                f.write_str("primitive exceeds 1,048,576 vertices or 6,291,456 indices")
            }
            Self::Degenerate { triangle } => write!(
                f,
                "primitive triangle {triangle} collapses in mesh coordinates"
            ),
        }
    }
}
impl Error for PrimitiveError {}

impl Mesh {
    /// Builds an XY grid with analytic +Z normals and tangent frames.
    pub fn subdivided_plane(options: PlaneOptions) -> Result<Self, PrimitiveError> {
        let [width, height] = options.size;
        dimension(width, "width")?;
        dimension(height, "height")?;
        let [columns, rows] = options.segments;
        segments(columns, "horizontal", 1)?;
        segments(rows, "vertical", 1)?;
        let mut data = Data::new(
            (u64::from(columns) + 1) * (u64::from(rows) + 1),
            u64::from(columns) * u64::from(rows) * 6,
        )?;
        for y in 0..=rows {
            let v = f64::from(y) / f64::from(rows);
            for x in 0..=columns {
                let u = f64::from(x) / f64::from(columns);
                data.vertex(
                    [
                        ((u - 0.5) * f64::from(width)) as f32,
                        ((0.5 - v) * f64::from(height)) as f32,
                        0.,
                    ],
                    [0., 0., 1.],
                    [u as f32, v as f32],
                    [1., 0., 0., -1.],
                );
            }
        }
        for y in 0..rows {
            for x in 0..columns {
                let a = y * (columns + 1) + x;
                let b = a + columns + 1;
                data.indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        data.finish()
    }

    /// Builds a UV sphere with exact seam positions, per-sector pole vertices,
    /// outward normals, and analytic tangents. No zero-area pole triangles are emitted.
    pub fn sphere(options: SphereOptions) -> Result<Self, PrimitiveError> {
        dimension(options.radius, "radius")?;
        let [sectors, rings] = options.segments;
        segments(sectors, "longitude", 3)?;
        segments(rings, "latitude", 2)?;
        let mut data = Data::new(
            (u64::from(rings) - 1) * (u64::from(sectors) + 1) + 2 * u64::from(sectors),
            6 * u64::from(sectors) * (u64::from(rings) - 1),
        )?;
        for y in 1..rings {
            let v = f64::from(y) / f64::from(rings);
            let (sin, cos) = (v * std::f64::consts::PI).sin_cos();
            for x in 0..=sectors {
                let (s, c) = azimuth(x, sectors);
                let normal = [sin * c, cos, sin * s];
                data.vertex(
                    normal.map(|n| (n * f64::from(options.radius)) as f32),
                    normal.map(|n| n as f32),
                    [x as f32 / sectors as f32, v as f32],
                    [-s as f32, 0., c as f32, 1.],
                );
            }
        }
        for y in 0..rings - 2 {
            data.side_row(y * (sectors + 1), (y + 1) * (sectors + 1), sectors);
        }
        for (north, row) in [(true, 0), (false, (rings - 2) * (sectors + 1))] {
            for x in 0..sectors {
                let u = (f64::from(x) + 0.5) / f64::from(sectors);
                let (s, c) = (u * std::f64::consts::TAU).sin_cos();
                let sign = if north { 1. } else { -1. };
                let tip = data.vertex(
                    [0., sign * options.radius, 0.],
                    [0., sign, 0.],
                    [u as f32, if north { 0. } else { 1. }],
                    [-s as f32, 0., c as f32, 1.],
                );
                data.indices.extend(if north {
                    [tip, row + x + 1, row + x]
                } else {
                    [row + x, row + x + 1, tip]
                });
            }
        }
        data.finish()
    }

    /// Builds a cylinder with smooth side normals and optional flat end caps.
    pub fn cylinder(options: CylinderOptions) -> Result<Self, PrimitiveError> {
        revolution(
            options.radius,
            options.height,
            options.segments,
            options.capped,
            false,
        )
    }

    /// Builds a cone with analytic sloped normals, per-sector tip vertices,
    /// and an optional flat base cap.
    pub fn cone(options: ConeOptions) -> Result<Self, PrimitiveError> {
        revolution(
            options.radius,
            options.height,
            options.segments,
            options.capped,
            true,
        )
    }
}

fn dimension(value: f32, parameter: &'static str) -> Result<(), PrimitiveError> {
    if value.is_finite() && value > 0. {
        Ok(())
    } else {
        Err(PrimitiveError::Dimension { parameter })
    }
}
fn segments(actual: u32, axis: &'static str, minimum: u32) -> Result<(), PrimitiveError> {
    if actual > 1_048_576 {
        return Err(PrimitiveError::TooLarge);
    }
    if actual >= minimum {
        Ok(())
    } else {
        Err(PrimitiveError::Segments {
            axis,
            minimum,
            actual,
        })
    }
}

// The seam repeats the first angle exactly instead of evaluating sin(2*pi).
fn azimuth(index: u32, sectors: u32) -> (f64, f64) {
    (f64::from(index % sectors) / f64::from(sectors) * std::f64::consts::TAU).sin_cos()
}

fn revolution(
    radius: f32,
    height: f32,
    divisions: [u32; 2],
    capped: bool,
    cone: bool,
) -> Result<Mesh, PrimitiveError> {
    dimension(radius, "radius")?;
    dimension(height, "height")?;
    let [sectors, rows] = divisions;
    segments(sectors, "radial", 3)?;
    segments(rows, "height", 1)?;
    let cap_count = if capped { if cone { 1 } else { 2 } } else { 0 };
    let vertices = (u64::from(rows) + u64::from(!cone)) * (u64::from(sectors) + 1)
        + if cone { u64::from(sectors) } else { 0 }
        + cap_count * (u64::from(sectors) + 2);
    let indices = (6 * u64::from(rows) - if cone { 3 } else { 0 }) * u64::from(sectors)
        + cap_count * 3 * u64::from(sectors);
    let mut data = Data::new(vertices, indices)?;
    let slope = if cone { f64::from(radius) } else { 0. };
    let length = f64::from(height).hypot(slope);
    let normal = |s: f64, c: f64| {
        [
            (f64::from(height) * c / length) as f32,
            (slope / length) as f32,
            (f64::from(height) * s / length) as f32,
        ]
    };
    for y in u32::from(cone)..=rows {
        let v = f64::from(y) / f64::from(rows);
        let r = f64::from(radius) * if cone { v } else { 1. };
        for x in 0..=sectors {
            let (s, c) = azimuth(x, sectors);
            data.vertex(
                [
                    (r * c) as f32,
                    ((0.5 - v) * f64::from(height)) as f32,
                    (r * s) as f32,
                ],
                normal(s, c),
                [x as f32 / sectors as f32, v as f32],
                [-s as f32, 0., c as f32, 1.],
            );
        }
    }
    for y in 0..rows - u32::from(cone) {
        data.side_row(y * (sectors + 1), (y + 1) * (sectors + 1), sectors);
    }
    if cone {
        for x in 0..sectors {
            let u = (f64::from(x) + 0.5) / f64::from(sectors);
            let (s, c) = (u * std::f64::consts::TAU).sin_cos();
            let tip = data.vertex(
                [0., height * 0.5, 0.],
                normal(s, c),
                [u as f32, 0.],
                [-s as f32, 0., c as f32, 1.],
            );
            data.indices.extend([tip, x + 1, x]);
        }
    }
    if capped {
        if !cone {
            data.cap(radius, height * 0.5, sectors, true);
        }
        data.cap(radius, -height * 0.5, sectors, false);
    }
    data.finish()
}

struct Data {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    tangents: Vec<[f32; 4]>,
}
impl Data {
    fn new(vertices: u64, indices: u64) -> Result<Self, PrimitiveError> {
        if vertices > 1_048_576 || indices > 6_291_456 {
            return Err(PrimitiveError::TooLarge);
        }
        Ok(Self {
            vertices: Vec::with_capacity(vertices as usize),
            indices: Vec::with_capacity(indices as usize),
            tangents: Vec::with_capacity(vertices as usize),
        })
    }
    fn vertex(
        &mut self,
        position: [f32; 3],
        normal: [f32; 3],
        uv: [f32; 2],
        tangent: [f32; 4],
    ) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            position,
            normal,
            uv,
        });
        self.tangents.push(tangent);
        index
    }
    fn side_row(&mut self, top: u32, bottom: u32, sectors: u32) {
        for x in 0..sectors {
            self.indices.extend([
                top + x,
                top + x + 1,
                bottom + x,
                top + x + 1,
                bottom + x + 1,
                bottom + x,
            ]);
        }
    }
    fn cap(&mut self, radius: f32, y: f32, sectors: u32, top: bool) {
        let sign = if top { 1. } else { -1. };
        let normal = [0., sign, 0.];
        let tangent = [1., 0., 0., -1.];
        let center = self.vertex([0., y, 0.], normal, [0.5, 0.5], tangent);
        let ring = self.vertices.len() as u32;
        for x in 0..=sectors {
            let (s, c) = azimuth(x, sectors);
            self.vertex(
                [
                    (f64::from(radius) * c) as f32,
                    y,
                    (f64::from(radius) * s) as f32,
                ],
                normal,
                [
                    (0.5 + 0.5 * c) as f32,
                    (0.5 + 0.5 * s * f64::from(sign)) as f32,
                ],
                tangent,
            );
        }
        for x in 0..sectors {
            self.indices.extend(if top {
                [center, ring + x + 1, ring + x]
            } else {
                [center, ring + x, ring + x + 1]
            });
        }
    }
    fn finish(self) -> Result<Mesh, PrimitiveError> {
        for (triangle, indices) in self.indices.chunks_exact(3).enumerate() {
            let [a, b, c] = std::array::from_fn::<_, 3, _>(|i| {
                self.vertices[indices[i] as usize].position.map(f64::from)
            });
            let ab: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let ac: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let area = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if area == [0.; 3] {
                return Err(PrimitiveError::Degenerate { triangle });
            }
        }
        Ok(Mesh::new(self.vertices, self.indices)
            .with_tangents(self.tangents)
            .expect("invalid analytic primitive tangents"))
    }
}
