use std::sync::Arc;

use super::{Mesh3d, visibility};

/// Triangle-corner expansion admission failure, before output allocation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeshExpansionError3d {
    /// The expanded mesh needs one vertex per source index.
    #[error("corner expansion needs {required} vertices, exceeding limit {limit}")]
    VertexLimit {
        /// Output vertex count, equal to the source index count.
        required: usize,
        /// Caller-supplied maximum output vertex count.
        limit: usize,
    },
    /// Sequential output indices cannot be represented by u32.
    #[error("corner expansion exceeds the u32 index range")]
    IndexRange,
}

impl Mesh3d {
    /// Copies each indexed corner into a distinct vertex in triangle order.
    /// Preserves stored attributes bit-for-bit, including tangents and their UV set.
    /// Unused vertices are omitted; degenerate triangles and winding are retained.
    /// Source indices map output vertices back to source vertices. `vertex_limit`
    /// bounds the output vertex count, not bytes or allocation failure.
    pub fn expand_corners(&self, vertex_limit: usize) -> Result<Arc<Self>, MeshExpansionError3d> {
        let count = self.indices.len();
        if count > vertex_limit {
            return Err(MeshExpansionError3d::VertexLimit {
                required: count,
                limit: vertex_limit,
            });
        }
        let count = u32::try_from(count).map_err(|_| MeshExpansionError3d::IndexRange)?;
        let vertices: Vec<_> = self
            .indices
            .iter()
            .map(|&source| self.vertices[source as usize])
            .collect();
        let indices: Vec<_> = (0..count).collect();
        Ok(Arc::new(Self {
            bounds: visibility::bounds(&vertices, &indices),
            vertices: vertices.into(),
            indices: indices.into(),
            tangents: self
                .tangents
                .as_ref()
                .map(|values| remap(values, &self.indices)),
            tangent_uv_set: self.tangent_uv_set,
            uv_sets: self
                .uv_sets
                .iter()
                .map(|(&set, values)| (set, remap(values, &self.indices)))
                .collect(),
            vertex_colors: self
                .vertex_colors
                .as_ref()
                .map(|values| remap(values, &self.indices)),
        }))
    }
}

fn remap<T: Copy>(values: &[T], indices: &[u32]) -> Arc<[T]> {
    indices
        .iter()
        .map(|&index| values[index as usize])
        .collect()
}
