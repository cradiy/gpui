use super::Mesh3d;
use std::sync::Arc;

/// Invalid texture-coordinate set. Vertex and component offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum UvSetError3d {
    /// Every vertex, including unused vertices, requires a coordinate.
    #[error("UV set {set} requires {expected} coordinates, received {actual}")]
    Count {
        /// Coordinate-set identifier.
        set: u32,
        /// Mesh vertex count.
        expected: usize,
        /// Supplied coordinate count.
        actual: usize,
    },
    /// A coordinate contains NaN or infinity.
    #[error("UV set {set}, vertex {vertex} has a non-finite component at {component}")]
    NonFinite {
        /// Coordinate-set identifier.
        set: u32,
        /// Vertex offset.
        vertex: usize,
        /// Coordinate component offset.
        component: usize,
    },
}

impl Mesh3d {
    /// Stored coordinate-set identifiers in ascending order, including set zero.
    pub fn uv_sets(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(0).chain(self.uv_sets.keys().copied())
    }

    /// A coordinate, or `None` for a missing set or out-of-range vertex.
    /// Set zero is the `uv` attribute in the vertex buffer.
    pub fn uv_at(&self, set: u32, vertex: usize) -> Option<[f32; 2]> {
        if set == 0 {
            self.vertices.get(vertex).map(|vertex| vertex.uv)
        } else {
            self.uv_sets.get(&set)?.get(vertex).copied()
        }
    }

    /// Attaches or replaces finite coordinates without changing positions or topology.
    /// Set identifiers may be sparse. Replacing the tangent basis's coordinate set
    /// removes tangents; other sets retain them. Snapshots share unchanged storage.
    pub fn with_uv_set(
        &self,
        set: u32,
        coordinates: Vec<[f32; 2]>,
    ) -> Result<Arc<Self>, UvSetError3d> {
        if coordinates.len() != self.vertices.len() {
            return Err(UvSetError3d::Count {
                set,
                expected: self.vertices.len(),
                actual: coordinates.len(),
            });
        }
        for (vertex, uv) in coordinates.iter().enumerate() {
            if let Some(component) = uv.iter().position(|value| !value.is_finite()) {
                return Err(UvSetError3d::NonFinite {
                    set,
                    vertex,
                    component,
                });
            }
        }
        let mut mesh = Self {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
            tangents: self.tangents.clone(),
            tangent_uv_set: self.tangent_uv_set,
            uv_sets: self.uv_sets.clone(),
            bounds: self.bounds,
        };
        if set == 0 {
            let mut vertices = self.vertices.to_vec();
            for (vertex, uv) in vertices.iter_mut().zip(coordinates) {
                vertex.uv = uv;
            }
            mesh.vertices = vertices.into();
        } else {
            mesh.uv_sets.insert(set, coordinates.into());
        }
        if self.tangent_uv_set() == Some(set) {
            mesh.tangents = None;
            mesh.tangent_uv_set = 0;
        }
        Ok(Arc::new(mesh))
    }
}
