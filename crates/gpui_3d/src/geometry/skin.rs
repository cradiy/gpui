use crate::{AffineTransform, Mesh, MeshUpdateError, Vertex};
use std::{fmt, sync::Arc};

mod palette;
pub use palette::SkinPalette;

/// One joint's contribution to a vertex. Joint indices address the inverse bind array.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinInfluence {
    pub joint: usize,
    pub weight: f32,
}

/// One retained contribution used by skin evaluation, normalized per vertex.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalizedSkinInfluence {
    /// Index into the binding's inverse bind matrices.
    pub joint: usize,
    /// Positive normalized weight in the evaluator's accumulation precision.
    pub weight: f64,
}

/// Invalid skin inputs or an unrepresentable sampled deformation. Offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkinError {
    EmptyJoints,
    EmptyVertices,
    TooLarge,
    VertexIndex {
        vertex: usize,
        vertex_count: usize,
    },
    JointIndex {
        vertex: usize,
        influence: usize,
        joint: usize,
    },
    InvalidWeight {
        vertex: usize,
        influence: usize,
    },
    MissingInfluences {
        vertex: usize,
    },
    JointCount {
        expected: usize,
        actual: usize,
    },
    VertexCount {
        expected: usize,
        actual: usize,
    },
    InvalidJointTransform {
        joint: usize,
    },
    PaletteBindingMismatch,
    InvalidVertexTransform {
        vertex: usize,
    },
    UnrepresentablePosition {
        vertex: usize,
    },
    Mesh(MeshUpdateError),
}

impl fmt::Display for SkinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyJoints => f.write_str("skin requires at least one joint"),
            Self::EmptyVertices => f.write_str("skin requires vertex influences"),
            Self::TooLarge => f.write_str("skin influence storage exceeds addressable size"),
            Self::VertexIndex {
                vertex,
                vertex_count,
            } => {
                write!(f, "skin vertex {vertex} is outside {vertex_count} vertices")
            }
            Self::JointIndex {
                vertex,
                influence,
                joint,
            } => write!(
                f,
                "skin vertex {vertex} influence {influence} references missing joint {joint}"
            ),
            Self::InvalidWeight { vertex, influence } => write!(
                f,
                "skin vertex {vertex} influence {influence} must have a finite nonnegative weight"
            ),
            Self::MissingInfluences { vertex } => {
                write!(f, "skin vertex {vertex} requires a positive joint weight")
            }
            Self::JointCount { expected, actual } => {
                write!(f, "expected {expected} joint transforms, received {actual}")
            }
            Self::VertexCount { expected, actual } => {
                write!(f, "expected {expected} skin vertices, received {actual}")
            }
            Self::InvalidJointTransform { joint } => {
                write!(f, "skin joint {joint} produces an invalid affine transform")
            }
            Self::PaletteBindingMismatch => {
                f.write_str("skin palette belongs to different joint bindings")
            }
            Self::InvalidVertexTransform { vertex } => write!(
                f,
                "skin vertex {vertex} has a singular or unrepresentable blended transform"
            ),
            Self::UnrepresentablePosition { vertex } => write!(
                f,
                "skinned position at vertex {vertex} is outside finite f32 range"
            ),
            Self::Mesh(source) => write!(f, "skinned mesh: {source}"),
        }
    }
}

impl std::error::Error for SkinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Mesh(source) => Some(source),
            _ => None,
        }
    }
}

/// Immutable linear-blend skin binding. Clones share joint bindings and normalized
/// per-vertex influences. Mesh vertex order must match the influence order.
#[derive(Clone, Debug)]
pub struct Skin {
    inverse_bind: Arc<[AffineTransform]>,
    offsets: Arc<[usize]>,
    influences: Arc<[NormalizedSkinInfluence]>,
}

impl Skin {
    /// Inverse bind matrices map mesh bind space into each joint's bind-local space.
    /// Each vertex needs a positive total weight. Finite nonnegative weights are
    /// normalized per vertex; zero entries are discarded after validation.
    /// There is no fixed joint count or per-vertex influence limit.
    pub fn new(
        inverse_bind_matrices: impl IntoIterator<Item = AffineTransform>,
        influences: impl IntoIterator<Item = impl IntoIterator<Item = SkinInfluence>>,
    ) -> Result<Self, SkinError> {
        let inverse_bind: Arc<[_]> = inverse_bind_matrices.into_iter().collect();
        if inverse_bind.is_empty() {
            return Err(SkinError::EmptyJoints);
        }
        let mut offsets = vec![0];
        let mut normalized = Vec::new();
        for (vertex, values) in influences.into_iter().enumerate() {
            let start = normalized.len();
            let mut sum = 0.;
            for (influence, value) in values.into_iter().enumerate() {
                if value.joint >= inverse_bind.len() {
                    return Err(SkinError::JointIndex {
                        vertex,
                        influence,
                        joint: value.joint,
                    });
                }
                if !value.weight.is_finite() || value.weight < 0. {
                    return Err(SkinError::InvalidWeight { vertex, influence });
                }
                if value.weight > 0. {
                    let weight = f64::from(value.weight);
                    sum += weight;
                    normalized.push(NormalizedSkinInfluence {
                        joint: value.joint,
                        weight,
                    });
                }
            }
            if sum == 0. {
                return Err(SkinError::MissingInfluences { vertex });
            }
            for influence in &mut normalized[start..] {
                influence.weight /= sum;
            }
            offsets.push(normalized.len());
        }
        if offsets.len() == 1 {
            return Err(SkinError::EmptyVertices);
        }
        Ok(Self {
            inverse_bind,
            offsets: offsets.into(),
            influences: normalized.into(),
        })
    }

    pub fn joint_count(&self) -> usize {
        self.inverse_bind.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn inverse_bind_matrices(&self) -> &[AffineTransform] {
        &self.inverse_bind
    }

    /// Reorders, duplicates or selects vertices using output-to-source indices.
    /// Normalized f64 weights and influence order are copied without conversion or
    /// renormalization. Joint bindings remain shared; joint indices do not change.
    /// Empty maps and missing source vertices are errors. Callers bound map sizes
    /// and retained results when processing untrusted topology.
    pub fn remap_vertices(&self, source_vertices: &[u32]) -> Result<Self, SkinError> {
        if source_vertices.is_empty() {
            return Err(SkinError::EmptyVertices);
        }
        let offsets_len = source_vertices
            .len()
            .checked_add(1)
            .ok_or(SkinError::TooLarge)?;
        let mut count = 0usize;
        for &vertex in source_vertices {
            count = count
                .checked_add(self.vertex_influences(vertex as usize)?.len())
                .ok_or(SkinError::TooLarge)?;
        }
        for (count, size) in [
            (offsets_len, std::mem::size_of::<usize>()),
            (count, std::mem::size_of::<NormalizedSkinInfluence>()),
        ] {
            if count
                .checked_mul(size)
                .is_none_or(|bytes| bytes > isize::MAX as usize)
            {
                return Err(SkinError::TooLarge);
            }
        }
        if source_vertices.len() == self.vertex_count()
            && source_vertices
                .iter()
                .enumerate()
                .all(|(index, &source)| index == source as usize)
        {
            return Ok(self.clone());
        }
        let mut offsets = Vec::with_capacity(offsets_len);
        let mut influences = Vec::with_capacity(count);
        offsets.push(0);
        for &vertex in source_vertices {
            let vertex = vertex as usize;
            influences.extend_from_slice(
                &self.influences[self.offsets[vertex]..self.offsets[vertex + 1]],
            );
            offsets.push(influences.len());
        }
        Ok(Self {
            inverse_bind: self.inverse_bind.clone(),
            offsets: offsets.into(),
            influences: influences.into(),
        })
    }

    /// Borrows the normalized contributions used to evaluate one vertex.
    /// Retains positive inputs in their original order, including repeated joints;
    /// zero-weight inputs are absent. No allocation or f32 conversion occurs.
    /// The index must be less than `vertex_count()`, including for unused vertices.
    pub fn vertex_influences(
        &self,
        vertex: usize,
    ) -> Result<&[NormalizedSkinInfluence], SkinError> {
        if vertex >= self.vertex_count() {
            return Err(SkinError::VertexIndex {
                vertex,
                vertex_count: self.vertex_count(),
            });
        }
        Ok(&self.influences[self.offsets[vertex]..self.offsets[vertex + 1]])
    }

    /// Builds each palette entry as `joint_to_mesh * inverse_bind`.
    /// Pass bind-space or morphed vertices, not a previous skinned result.
    /// The CPU result has fresh bounds and queries with shared triangle storage.
    /// Normals use the inverse transpose of the blended matrix; tangents use its
    /// linear part with reflection-aware handedness. Singular blended matrices
    /// and triangles with mixed tangent handedness return errors.
    pub fn evaluate(&self, mesh: &Mesh, joints: &[AffineTransform]) -> Result<Mesh, SkinError> {
        self.evaluate_world(mesh, AffineTransform::IDENTITY, joints)
    }

    /// Samples world-space joint poses into the output mesh's local space:
    /// `inverse(mesh_world) * joint_world * inverse_bind`.
    /// Render the result under `mesh_world`; joint hierarchy evaluation belongs
    /// to the caller. All joint transforms must be supplied in binding order.
    pub fn evaluate_world(
        &self,
        mesh: &Mesh,
        mesh_world: AffineTransform,
        joint_world: &[AffineTransform],
    ) -> Result<Mesh, SkinError> {
        if mesh.vertex_count() != self.vertex_count() {
            return Err(SkinError::VertexCount {
                expected: self.vertex_count(),
                actual: mesh.vertex_count(),
            });
        }
        let palette = self.palette(mesh_world, joint_world)?;
        self.evaluate_with_palette(mesh, &palette)
    }

    /// Evaluates bind-space or morphed vertices with a retained mesh-local palette.
    /// The palette must retain this binding's inverse-bind allocation, including
    /// clones and remapped influences. Matrix blending and output validation are
    /// identical to `evaluate_world`; previous results remain unchanged.
    pub fn evaluate_with_palette(
        &self,
        mesh: &Mesh,
        palette: &SkinPalette,
    ) -> Result<Mesh, SkinError> {
        if mesh.vertex_count() != self.vertex_count() {
            return Err(SkinError::VertexCount {
                expected: self.vertex_count(),
                actual: mesh.vertex_count(),
            });
        }
        palette.validate_binding(self)?;
        let palette = palette.matrices();
        let mut vertices = Vec::with_capacity(mesh.vertex_count());
        let mut tangents = mesh
            .tangents()
            .map(|_| Vec::with_capacity(mesh.vertex_count()));
        for (vertex, base) in mesh.vertices().iter().enumerate() {
            let mut blended = [[0_f64; 4]; 4];
            for &NormalizedSkinInfluence { joint, weight } in
                &self.influences[self.offsets[vertex]..self.offsets[vertex + 1]]
            {
                for (c, column) in blended.iter_mut().enumerate() {
                    for (r, value) in column.iter_mut().enumerate().take(3) {
                        *value += weight * f64::from(palette[joint][c][r]);
                    }
                }
            }
            blended[3][3] = 1.;
            let matrix = blended.map(|column| column.map(|value| value as f32));
            let transform = AffineTransform::from_matrix(matrix)
                .map_err(|_| SkinError::InvalidVertexTransform { vertex })?;
            let position = apply(matrix, base.position, 1.).map(|v| v as f32);
            if !position.iter().all(|v| v.is_finite()) {
                return Err(SkinError::UnrepresentablePosition { vertex });
            }
            vertices.push(Vertex {
                position,
                normal: direction(transform.normal_matrix(), base.normal),
                uv: base.uv,
            });
            if let Some(tangents) = &mut tangents {
                let [x, y, z, w] = mesh.tangents().unwrap()[vertex];
                let [x, y, z] = direction(matrix, [x, y, z]);
                tangents.push([x, y, z, w * determinant_sign(matrix)]);
            }
        }
        mesh.with_vertices(vertices, tangents)
            .map_err(SkinError::Mesh)
    }
}

fn apply(matrix: [[f32; 4]; 4], value: [f32; 3], w: f64) -> [f64; 3] {
    std::array::from_fn(|r| {
        (0..3)
            .map(|c| f64::from(matrix[c][r]) * f64::from(value[c]))
            .sum::<f64>()
            + f64::from(matrix[3][r]) * w
    })
}

fn direction(matrix: [[f32; 4]; 4], value: [f32; 3]) -> [f32; 3] {
    let value = apply(matrix, value, 0.);
    let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
    if length > 0. {
        value.map(|v| (v / length) as f32)
    } else {
        [0.; 3]
    }
}

fn determinant_sign(matrix: [[f32; 4]; 4]) -> f32 {
    let [a, b, c, _] = matrix.map(|v| v.map(f64::from));
    (a[0] * (b[1] * c[2] - b[2] * c[1]) - b[0] * (a[1] * c[2] - a[2] * c[1])
        + c[0] * (a[1] * b[2] - a[2] * b[1]))
        .signum() as f32
}
