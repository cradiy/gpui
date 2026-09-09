use crate::{MeshError, MeshUpdateError, TangentError, Vertex, math, spatial::bvh};
use gpui::Mesh3d;
use std::sync::{Arc, OnceLock};

/// Shared immutable indexed geometry.
#[derive(Clone, Debug)]
pub struct Mesh(pub(crate) Arc<Mesh3d>, pub(crate) Arc<OnceLock<bvh::Bvh>>);
impl Mesh {
    /// Creates counterclockwise triangles from mesh-local vertex data.
    /// Panics for invalid geometry; use `try_new` for fallible construction.
    #[track_caller]
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self(Mesh3d::new(vertices, indices), Arc::default())
    }
    /// Validates nonempty indexed triangles with finite vertex attributes.
    /// Preserves unused vertices, degenerate triangles and input ordering.
    pub fn try_new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Result<Self, MeshError> {
        Mesh3d::try_new(vertices, indices).map(|geometry| Self(geometry, Arc::default()))
    }
    /// Borrowed mesh-local vertices, including unreferenced vertices.
    pub fn vertices(&self) -> &[Vertex] {
        self.0.vertices()
    }
    /// Borrowed triangle indices in input order.
    pub fn indices(&self) -> &[u32] {
        self.0.indices()
    }
    /// Mesh-local tangent XYZ and handedness W, if supplied.
    pub fn tangents(&self) -> Option<&[[f32; 4]]> {
        self.0.tangents()
    }
    /// Attaches validated tangents without changing geometry, triangle identities
    /// or the source mesh. XYZ is normalized and orthogonalized against normals;
    /// W must be -1 or +1, constant within each triangle.
    pub fn with_tangents(&self, tangents: Vec<[f32; 4]>) -> Result<Self, TangentError> {
        self.0
            .with_tangents(tangents)
            .map(|mesh| Self(mesh, self.1.clone()))
    }
    /// Returns a fixed-topology snapshot with replacement positions, normals and UVs.
    /// Vertex count and triangle identities are preserved; index storage is shared.
    /// Supply tangents for the replacement normals, or `None` to omit them.
    /// The new snapshot has independent bounds and a fresh lazy query index.
    pub fn with_vertices(
        &self,
        vertices: Vec<Vertex>,
        tangents: Option<Vec<[f32; 4]>>,
    ) -> Result<Self, MeshUpdateError> {
        self.0
            .with_vertices(vertices, tangents)
            .map(|mesh| Self(mesh, Arc::default()))
    }
    pub fn vertex_count(&self) -> usize {
        self.vertices().len()
    }
    pub fn index_count(&self) -> usize {
        self.indices().len()
    }
    /// Number of indexed triangles, including degenerate triangles.
    pub fn triangle_count(&self) -> usize {
        self.index_count() / 3
    }
    /// Unit XY plane centered at the origin, facing positive Z.
    pub fn plane() -> Self {
        static PLANE: OnceLock<Mesh> = OnceLock::new();
        PLANE
            .get_or_init(|| {
                Self(
                    face_mesh(&[([0., 0., 0.], [1., 0., 0.], [0., 1., 0.])]),
                    Arc::default(),
                )
            })
            .clone()
    }
    /// Unit cube centered at the origin, with per-face normals and UVs.
    pub fn cube() -> Self {
        static CUBE: OnceLock<Mesh> = OnceLock::new();
        CUBE.get_or_init(|| {
            Self(
                face_mesh(&[
                    ([0., 0., 0.5], [1., 0., 0.], [0., 1., 0.]),
                    ([0., 0., -0.5], [-1., 0., 0.], [0., 1., 0.]),
                    ([0.5, 0., 0.], [0., 0., -1.], [0., 1., 0.]),
                    ([-0.5, 0., 0.], [0., 0., 1.], [0., 1., 0.]),
                    ([0., 0.5, 0.], [1., 0., 0.], [0., 0., -1.]),
                    ([0., -0.5, 0.], [1., 0., 0.], [0., 0., 1.]),
                ]),
                Arc::default(),
            )
        })
        .clone()
    }
}

type Face = ([f32; 3], [f32; 3], [f32; 3]);
fn face_mesh(faces: &[Face]) -> Arc<Mesh3d> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut tangents = Vec::new();
    for &(center, right, up) in faces {
        let first = vertices.len() as u32;
        for (x, y, uv) in [
            (-0.5, -0.5, [0., 1.]),
            (0.5, -0.5, [1., 1.]),
            (0.5, 0.5, [1., 0.]),
            (-0.5, 0.5, [0., 0.]),
        ] {
            vertices.push(Vertex {
                position: std::array::from_fn(|i| center[i] + right[i] * x + up[i] * y),
                normal: math::cross(right, up),
                uv,
            });
            tangents.push([right[0], right[1], right[2], -1.]);
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| i + first));
    }
    Mesh3d::new(vertices, indices)
        .with_tangents(tangents)
        .expect("invalid face tangents")
}
