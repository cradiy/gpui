use std::{collections::HashMap, error::Error, fmt};

use bevy_mikktspace::{Geometry, TangentSpace};

use crate::{Mesh, TangentError, Vertex};

/// A tangent-bearing mesh and its output-to-source vertex mapping.
#[derive(Clone, Debug)]
pub struct GeneratedTangents {
    mesh: Mesh,
    source_vertices: Vec<u32>,
}

impl GeneratedTangents {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    /// Source vertex index for each output vertex. Use this to remap external
    /// vertex attributes, morph deltas, and skin influences before deformation.
    pub fn source_vertices(&self) -> &[u32] {
        &self.source_vertices
    }

    pub fn into_parts(self) -> (Mesh, Vec<u32>) {
        (self.mesh, self.source_vertices)
    }
}

/// Tangent generation failure. Triangle, corner, and source vertex offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TangentGenerationError {
    /// An indexed vertex has a zero normal.
    InvalidNormal { vertex: usize },
    /// A triangle has zero geometric area.
    DegenerateGeometry { triangle: usize },
    /// A triangle has zero UV area.
    DegenerateUv { triangle: usize },
    /// Intermediate geometry or UV calculations exceed the f32 numeric range.
    Unrepresentable { triangle: usize },
    /// No finite, nonzero tangent orthogonal to the normal was generated.
    InvalidBasis { triangle: usize, corner: usize },
    /// Generation exceeded the supported u32 vertex/index range.
    TooLarge,
    /// The generated mesh failed tangent validation.
    Tangents(TangentError),
}

impl fmt::Display for TangentGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNormal { vertex } => write!(f, "zero normal at vertex {vertex}"),
            Self::DegenerateGeometry { triangle } => {
                write!(f, "zero geometric area at triangle {triangle}")
            }
            Self::DegenerateUv { triangle } => write!(f, "zero UV area at triangle {triangle}"),
            Self::Unrepresentable { triangle } => write!(
                f,
                "unrepresentable tangent calculation at triangle {triangle}"
            ),
            Self::InvalidBasis { triangle, corner } => {
                write!(f, "invalid tangent at triangle {triangle}, corner {corner}")
            }
            Self::TooLarge => f.write_str("tangent generation exceeds the u32 index range"),
            Self::Tangents(error) => error.fmt(f),
        }
    }
}

impl Error for TangentGenerationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Tangents(error) => Some(error),
            _ => None,
        }
    }
}

impl Mesh {
    /// Generates MikkTSpace tangents synchronously from positions, normals and UVs.
    ///
    /// Splits shared vertices when face-corner tangents differ, including mirrored
    /// UV seams. Output vertices follow first use; unused vertices are omitted.
    /// Vertex attributes, triangle order and winding are preserved. Existing tangents
    /// are replaced; the source mesh is unchanged. The result includes a vertex map
    /// for remapping external attributes. Bounds exclude omitted vertices.
    ///
    /// Indexed normals must be nonzero; a normalized copy is used by the algorithm.
    /// Zero geometric/UV area and unrepresentable f32 intermediates return errors;
    /// triangles are never silently dropped and no arbitrary basis is substituted.
    pub fn generate_tangents(&self) -> Result<GeneratedTangents, TangentGenerationError> {
        if self.index_count() > u32::MAX as usize || self.triangle_count() > usize::MAX / 4 {
            return Err(TangentGenerationError::TooLarge);
        }
        for (triangle, indices) in self.indices().chunks_exact(3).enumerate() {
            for &index in indices {
                if normalized(self.vertices()[index as usize].normal).is_none() {
                    return Err(TangentGenerationError::InvalidNormal {
                        vertex: index as usize,
                    });
                }
            }
            validate_triangle(
                std::array::from_fn(|i| self.vertices()[indices[i] as usize]),
                triangle,
            )?;
        }

        let mut geometry = TangentGeometry {
            mesh: self,
            corners: vec![None; self.index_count()],
        };
        // The dependency's error type is uninhabited.
        bevy_mikktspace::generate_tangents(&mut geometry)
            .expect("MikkTSpace returned an uninhabited error");

        let mut vertices = Vec::new();
        let mut tangents = Vec::new();
        let mut indices = Vec::with_capacity(self.index_count());
        let mut source_vertices = Vec::new();
        let mut shared = HashMap::new();
        for (offset, (&source, tangent)) in self.indices().iter().zip(geometry.corners).enumerate()
        {
            let vertex = self.vertices()[source as usize];
            let tangent = tangent
                .and_then(|tangent| orthogonalized(tangent, vertex.normal))
                .ok_or(TangentGenerationError::InvalidBasis {
                    triangle: offset / 3,
                    corner: offset % 3,
                })?;
            let key = (
                source,
                tangent.map(|value| if value == 0. { 0 } else { value.to_bits() }),
            );
            let index = *shared.entry(key).or_insert_with(|| {
                let index = vertices.len() as u32;
                vertices.push(vertex);
                tangents.push(tangent);
                source_vertices.push(source);
                index
            });
            indices.push(index);
        }
        let mesh = Mesh::new(vertices, indices)
            .with_tangents(tangents)
            .map_err(TangentGenerationError::Tangents)?;
        Ok(GeneratedTangents {
            mesh,
            source_vertices,
        })
    }
}

struct TangentGeometry<'a> {
    mesh: &'a Mesh,
    corners: Vec<Option<[f32; 4]>>,
}

impl TangentGeometry<'_> {
    fn vertex(&self, face: usize, corner: usize) -> Vertex {
        self.mesh.vertices()[self.mesh.indices()[face * 3 + corner] as usize]
    }
}

impl Geometry for TangentGeometry<'_> {
    fn num_faces(&self) -> usize {
        self.mesh.triangle_count()
    }
    fn num_vertices_of_face(&self, _: usize) -> usize {
        3
    }
    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.vertex(face, vert).position
    }
    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        normalized(self.vertex(face, vert).normal).expect("validated normal")
    }
    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.vertex(face, vert).uv
    }
    fn set_tangent(&mut self, tangent: Option<TangentSpace>, face: usize, vert: usize) {
        self.corners[face * 3 + vert] = tangent.map(|space| space.tangent_encoded());
    }
}

fn normalized(value: [f32; 3]) -> Option<[f32; 3]> {
    let length = value
        .iter()
        .map(|&v| f64::from(v).powi(2))
        .sum::<f64>()
        .sqrt();
    (length > 0.).then(|| value.map(|v| (f64::from(v) / length) as f32))
}

fn orthogonalized(tangent: [f32; 4], normal: [f32; 3]) -> Option<[f32; 4]> {
    if !tangent.iter().all(|v| v.is_finite()) || tangent[3].abs() != 1. {
        return None;
    }
    let n = normal.map(f64::from);
    let n_length = n.iter().map(|v| v * v).sum::<f64>().sqrt();
    let n = n.map(|v| v / n_length);
    let t = [tangent[0], tangent[1], tangent[2]].map(f64::from);
    let dot = t.iter().zip(n).map(|(t, n)| t * n).sum::<f64>();
    let projected: [f64; 3] = std::array::from_fn(|i| t[i] - dot * n[i]);
    let length = projected.iter().map(|v| v * v).sum::<f64>().sqrt();
    let original_length = t.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length <= original_length * 1e-6 {
        return None;
    }
    Some([
        (projected[0] / length) as f32,
        (projected[1] / length) as f32,
        (projected[2] / length) as f32,
        tangent[3],
    ])
}

fn validate_triangle(vertices: [Vertex; 3], triangle: usize) -> Result<(), TangentGenerationError> {
    let edges: [[f64; 3]; 2] = [1, 2].map(|v| {
        std::array::from_fn(|i| {
            f64::from(vertices[v].position[i]) - f64::from(vertices[0].position[i])
        })
    });
    let area: [f64; 3] = std::array::from_fn(|i| {
        edges[0][(i + 1) % 3] * edges[1][(i + 2) % 3]
            - edges[0][(i + 2) % 3] * edges[1][(i + 1) % 3]
    });
    if area == [0.; 3] {
        return Err(TangentGenerationError::DegenerateGeometry { triangle });
    }
    let uv: [[f64; 2]; 2] = [1, 2].map(|v| {
        std::array::from_fn(|i| f64::from(vertices[v].uv[i]) - f64::from(vertices[0].uv[i]))
    });
    if uv[0][0] * uv[1][1] - uv[0][1] * uv[1][0] == 0. {
        return Err(TangentGenerationError::DegenerateUv { triangle });
    }
    let edges = edges.map(|v| v.map(|v| v as f32));
    let uv = uv.map(|v| v.map(|v| v as f32));
    let determinant = uv[0][0] * uv[1][1] - uv[0][1] * uv[1][0];
    let s: [f32; 3] = std::array::from_fn(|i| edges[0][i] * uv[1][1] - edges[1][i] * uv[0][1]);
    let t: [f32; 3] = std::array::from_fn(|i| edges[1][i] * uv[0][0] - edges[0][i] * uv[1][0]);
    let opposite: [f32; 3] =
        std::array::from_fn(|i| vertices[2].position[i] - vertices[1].position[i]);
    let valid = determinant.is_normal()
        && edges
            .into_iter()
            .chain([opposite])
            .all(|v| v.iter().map(|v| v * v).sum::<f32>().is_normal())
        && [s, t].into_iter().all(|v| {
            let squared = v.iter().map(|v| v * v).sum::<f32>();
            squared.is_normal() && (squared.sqrt() / determinant.abs()).is_finite()
        });
    if !valid {
        return Err(TangentGenerationError::Unrepresentable { triangle });
    }
    Ok(())
}
