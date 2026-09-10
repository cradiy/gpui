use std::{collections::HashMap, error::Error, fmt};

use bevy_mikktspace::{Geometry, TangentSpace};

use crate::{Mesh, TangentError, Vertex};

/// Handling of degenerate triangles and undefined tangent frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TangentGenerationMode {
    /// Reject zero-area triangles and undefined frames.
    #[default]
    Strict,
    /// Let MikkTSpace inherit neighboring tangent frames. Corners without a usable
    /// inherited frame still return an error; no default basis is substituted.
    Inherit,
    /// Inherit neighboring frames, then repair undefined corners using a triangle
    /// derivative or a deterministic normal-orthogonal basis. Repairs are reported.
    Repair,
}

/// Source of a repaired corner's tangent direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TangentRepairKind {
    /// The triangle's position/UV derivative, projected against the vertex normal.
    TriangleDerivative,
    /// The least-aligned coordinate axis, projected against the vertex normal.
    /// This is a convention, not a recovered UV direction.
    OrthonormalBasis,
}

/// A repaired corner, addressed in the original triangle order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TangentRepair {
    pub triangle: usize,
    pub corner: usize,
    pub kind: TangentRepairKind,
}

/// A tangent-bearing mesh and its output-to-source vertex mapping.
#[derive(Clone, Debug)]
pub struct GeneratedTangents {
    mesh: Mesh,
    source_vertices: Vec<u32>,
    repairs: Vec<TangentRepair>,
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

    /// Corners whose undefined MikkTSpace result required explicit repair.
    /// Neighbor-inherited MikkTSpace results are not repairs.
    pub fn repairs(&self) -> &[TangentRepair] {
        &self.repairs
    }

    pub fn into_parts(self) -> (Mesh, Vec<u32>) {
        (self.mesh, self.source_vertices)
    }
}

/// Tangent generation failure. Triangle, corner, and source vertex offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TangentGenerationError {
    /// The requested coordinate set is absent.
    MissingUvSet { set: u32 },
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
            Self::MissingUvSet { set } => write!(f, "missing UV set {set} for tangent generation"),
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
        self.generate_tangents_with_mode(TangentGenerationMode::Strict)
    }

    /// Generates tangents with explicit handling of degenerate inputs and frames.
    /// Retains every triangle and the same output-to-source mapping contract as
    /// [`Self::generate_tangents`]. Invalid normals, unrepresentable calculations,
    /// and incompatible triangle handedness remain errors in every mode.
    pub fn generate_tangents_with_mode(
        &self,
        mode: TangentGenerationMode,
    ) -> Result<GeneratedTangents, TangentGenerationError> {
        self.generate_tangents_for_uv_set(0, mode)
    }

    /// Generates MikkTSpace frames using an existing coordinate set and records
    /// that set on the result. All coordinate sets and the original `Vertex::uv`
    /// are preserved through vertex splitting. Error policy matches
    /// [`Self::generate_tangents_with_mode`].
    pub fn generate_tangents_for_uv_set(
        &self,
        set: u32,
        mode: TangentGenerationMode,
    ) -> Result<GeneratedTangents, TangentGenerationError> {
        if self.uv_at(set, 0).is_none() {
            return Err(TangentGenerationError::MissingUvSet { set });
        }
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
                std::array::from_fn(|i| vertex_with_uv(self, indices[i] as usize, set)),
                triangle,
                mode,
            )?;
        }

        let mut geometry = TangentGeometry {
            mesh: self,
            uv_set: set,
            corners: vec![None; self.index_count()],
        };
        // The dependency's error type is uninhabited.
        bevy_mikktspace::generate_tangents(&mut geometry)
            .expect("MikkTSpace returned an uninhabited error");

        let mut repairs = Vec::new();
        if mode == TangentGenerationMode::Repair {
            for (triangle, (face, corners)) in self
                .indices()
                .chunks_exact(3)
                .zip(geometry.corners.chunks_exact_mut(3))
                .enumerate()
            {
                let vertices = std::array::from_fn(|i| vertex_with_uv(self, face[i] as usize, set));
                let derivative = triangle_derivative(vertices);
                let handedness = corners
                    .iter()
                    .zip(vertices)
                    .find_map(|(&t, v)| t.and_then(|t| orthogonalized(t, v.normal)))
                    .map(|t| t[3])
                    .unwrap_or_else(|| derivative.map_or(1., |t| t[3]));
                for (corner, (tangent, vertex)) in corners.iter_mut().zip(vertices).enumerate() {
                    if tangent
                        .and_then(|t| orthogonalized(t, vertex.normal))
                        .is_some()
                    {
                        continue;
                    }
                    let (mut repaired, kind) =
                        match derivative.and_then(|t| orthogonalized(t, vertex.normal)) {
                            Some(t) => (t, TangentRepairKind::TriangleDerivative),
                            None => (
                                normal_basis(vertex.normal),
                                TangentRepairKind::OrthonormalBasis,
                            ),
                        };
                    repaired[3] = handedness;
                    *tangent = Some(repaired);
                    repairs.push(TangentRepair {
                        triangle,
                        corner,
                        kind,
                    });
                }
            }
        }

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
        let mesh = self
            .remap_attributes(Mesh::new(vertices, indices), &source_vertices)
            .with_tangents_for_uv_set(set, tangents)
            .map_err(TangentGenerationError::Tangents)?;
        Ok(GeneratedTangents {
            mesh,
            source_vertices,
            repairs,
        })
    }
}

struct TangentGeometry<'a> {
    mesh: &'a Mesh,
    uv_set: u32,
    corners: Vec<Option<[f32; 4]>>,
}

impl TangentGeometry<'_> {
    fn vertex(&self, face: usize, corner: usize) -> Vertex {
        vertex_with_uv(
            self.mesh,
            self.mesh.indices()[face * 3 + corner] as usize,
            self.uv_set,
        )
    }
}

fn vertex_with_uv(mesh: &Mesh, index: usize, set: u32) -> Vertex {
    let mut vertex = mesh.vertices()[index];
    vertex.uv = mesh.uv_at(set, index).expect("validated coordinate set");
    vertex
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

fn triangle_derivative(vertices: [Vertex; 3]) -> Option<[f32; 4]> {
    let uv: [[f64; 2]; 2] = [1, 2].map(|v| {
        std::array::from_fn(|i| f64::from(vertices[v].uv[i]) - f64::from(vertices[0].uv[i]))
    });
    let determinant = uv[0][0] * uv[1][1] - uv[0][1] * uv[1][0];
    if determinant == 0. {
        return None;
    }
    let direction: [f64; 3] = std::array::from_fn(|i| {
        ((f64::from(vertices[1].position[i]) - f64::from(vertices[0].position[i])) * uv[1][1]
            - (f64::from(vertices[2].position[i]) - f64::from(vertices[0].position[i])) * uv[0][1])
            * determinant.signum()
    });
    let length = direction.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length == 0. {
        return None;
    }
    let t = direction.map(|v| (v / length) as f32);
    Some([t[0], t[1], t[2], determinant.signum() as f32])
}

fn normal_basis(normal: [f32; 3]) -> [f32; 4] {
    let axis = (0..3)
        .min_by(|&a, &b| normal[a].abs().total_cmp(&normal[b].abs()))
        .unwrap();
    let mut tangent = [0., 0., 0., 1.];
    tangent[axis] = 1.;
    orthogonalized(tangent, normal).expect("nonzero normal and least-aligned axis")
}

fn validate_triangle(
    vertices: [Vertex; 3],
    triangle: usize,
    mode: TangentGenerationMode,
) -> Result<(), TangentGenerationError> {
    let edges: [[f64; 3]; 2] = [1, 2].map(|v| {
        std::array::from_fn(|i| {
            f64::from(vertices[v].position[i]) - f64::from(vertices[0].position[i])
        })
    });
    let area: [f64; 3] = std::array::from_fn(|i| {
        edges[0][(i + 1) % 3] * edges[1][(i + 2) % 3]
            - edges[0][(i + 2) % 3] * edges[1][(i + 1) % 3]
    });
    let zero_geometry = area == [0.; 3];
    if zero_geometry && mode == TangentGenerationMode::Strict {
        return Err(TangentGenerationError::DegenerateGeometry { triangle });
    }
    let uv: [[f64; 2]; 2] = [1, 2].map(|v| {
        std::array::from_fn(|i| f64::from(vertices[v].uv[i]) - f64::from(vertices[0].uv[i]))
    });
    let zero_uv = uv[0][0] * uv[1][1] - uv[0][1] * uv[1][0] == 0.;
    if zero_uv && mode == TangentGenerationMode::Strict {
        return Err(TangentGenerationError::DegenerateUv { triangle });
    }
    let edges = edges.map(|v| v.map(|v| v as f32));
    let uv = uv.map(|v| v.map(|v| v as f32));
    let determinant = uv[0][0] * uv[1][1] - uv[0][1] * uv[1][0];
    let s: [f32; 3] = std::array::from_fn(|i| edges[0][i] * uv[1][1] - edges[1][i] * uv[0][1]);
    let t: [f32; 3] = std::array::from_fn(|i| edges[1][i] * uv[0][0] - edges[0][i] * uv[1][0]);
    let opposite: [f32; 3] =
        std::array::from_fn(|i| vertices[2].position[i] - vertices[1].position[i]);
    let inherited = zero_geometry || zero_uv;
    let valid = (determinant.is_normal() || (zero_uv && determinant == 0.))
        && edges.into_iter().chain([opposite]).all(|v| {
            v.iter().map(|v| v * v).sum::<f32>().is_normal() || (inherited && v == [0.; 3])
        })
        && [s, t].into_iter().all(|v| {
            let squared = v.iter().map(|v| v * v).sum::<f32>();
            if inherited {
                squared.is_normal() || v == [0.; 3]
            } else {
                squared.is_normal() && (squared.sqrt() / determinant.abs()).is_finite()
            }
        });
    if !valid {
        return Err(TangentGenerationError::Unrepresentable { triangle });
    }
    Ok(())
}
