use std::{collections::HashMap, error::Error, fmt};

use crate::{Mesh, MeshError};

/// How indexed triangle normals are assigned to vertices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NormalMode {
    /// One normal per triangle, splitting shared vertices across different normals.
    Flat,
    /// Area-weighted normals at shared source indices. Coincident positions are
    /// not welded, so authored vertex splits remain shading boundaries.
    Smooth,
}

/// A mesh with generated normals and its output-to-source vertex mapping.
#[derive(Clone, Debug)]
pub struct GeneratedNormals {
    mesh: Mesh,
    source_vertices: Vec<u32>,
}

impl GeneratedNormals {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    /// Source index for each output vertex. Remap external vertex attributes,
    /// skin influences, and morph deltas through this map after generation.
    pub fn source_vertices(&self) -> &[u32] {
        &self.source_vertices
    }

    pub fn into_parts(self) -> (Mesh, Vec<u32>) {
        (self.mesh, self.source_vertices)
    }
}

/// Normal generation failure, with zero-based source indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormalGenerationError {
    DegenerateGeometry {
        triangle: usize,
    },
    /// Incident area-weighted normals cancel or cannot be normalized.
    InvalidNormal {
        vertex: usize,
    },
    TooLarge,
    Mesh(MeshError),
}

impl fmt::Display for NormalGenerationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DegenerateGeometry { triangle } => {
                write!(f, "zero geometric area at triangle {triangle}")
            }
            Self::InvalidNormal { vertex } => {
                write!(f, "undefined smooth normal at vertex {vertex}")
            }
            Self::TooLarge => f.write_str("normal generation exceeds the u32 index range"),
            Self::Mesh(error) => error.fmt(f),
        }
    }
}

impl Error for NormalGenerationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Mesh(error) => Some(error),
            _ => None,
        }
    }
}

impl Mesh {
    /// Generates normals synchronously from indexed positions and winding.
    ///
    /// Flat normals split shared vertices where normalized face normals differ.
    /// Smooth normals sum unnormalized face cross products at each source index,
    /// giving area weighting without welding coincident positions. Degenerate
    /// triangles and cancelling smooth normals are errors, not skipped geometry.
    ///
    /// Triangle order, winding, positions and UVs are preserved. Output vertices
    /// follow first use; unreferenced vertices are omitted. Existing normals are
    /// replaced and tangents are removed. Inputs remain unchanged on success or
    /// failure. The output map supports remapping deformation and other attributes.
    pub fn generate_normals(
        &self,
        mode: NormalMode,
    ) -> Result<GeneratedNormals, NormalGenerationError> {
        if self.index_count() > u32::MAX as usize {
            return Err(NormalGenerationError::TooLarge);
        }
        let mut faces = if mode == NormalMode::Flat {
            Vec::with_capacity(self.triangle_count())
        } else {
            Vec::new()
        };
        let mut sums = if mode == NormalMode::Smooth {
            vec![[0_f64; 3]; self.vertex_count()]
        } else {
            Vec::new()
        };
        for (triangle, indices) in self.indices().chunks_exact(3).enumerate() {
            let [a, b, c] = std::array::from_fn(|i| {
                self.vertices()[indices[i] as usize].position.map(f64::from)
            });
            let u: [f64; 3] = std::array::from_fn(|i| b[i] - a[i]);
            let v: [f64; 3] = std::array::from_fn(|i| c[i] - a[i]);
            let cross = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let normal =
                normalized(cross).ok_or(NormalGenerationError::DegenerateGeometry { triangle })?;
            if mode == NormalMode::Smooth {
                for &index in indices {
                    for (sum, component) in sums[index as usize].iter_mut().zip(cross) {
                        *sum += component;
                    }
                }
            } else {
                faces.push(normal);
            }
        }

        let mut vertices = Vec::new();
        let mut indices = Vec::with_capacity(self.index_count());
        let mut source_vertices = Vec::new();
        let mut shared = HashMap::new();
        for (offset, &source) in self.indices().iter().enumerate() {
            let normal = match mode {
                NormalMode::Flat => faces[offset / 3],
                NormalMode::Smooth => normalized(sums[source as usize]).ok_or(
                    NormalGenerationError::InvalidNormal {
                        vertex: source as usize,
                    },
                )?,
            };
            let key = (
                source,
                normal.map(|value| if value == 0. { 0 } else { value.to_bits() }),
            );
            let index = *shared.entry(key).or_insert_with(|| {
                let index = vertices.len() as u32;
                let mut vertex = self.vertices()[source as usize];
                vertex.normal = normal;
                vertices.push(vertex);
                source_vertices.push(source);
                index
            });
            indices.push(index);
        }
        let mesh = Mesh::try_new(vertices, indices).map_err(NormalGenerationError::Mesh)?;
        let mesh = self.remap_uv_sets(mesh, &source_vertices);
        Ok(GeneratedNormals {
            mesh,
            source_vertices,
        })
    }
}

fn normalized(value: [f64; 3]) -> Option<[f32; 3]> {
    let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
    (length.is_finite() && length > 0.).then(|| value.map(|v| (v / length) as f32))
}
