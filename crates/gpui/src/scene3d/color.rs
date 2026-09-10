use super::Mesh3d;
use std::sync::Arc;

/// Invalid linear vertex RGBA data. Offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VertexColorError3d {
    /// Every vertex, including unused vertices, requires a color.
    #[error("expected {expected} vertex colors, received {actual}")]
    Count {
        /// Mesh vertex count.
        expected: usize,
        /// Supplied color count.
        actual: usize,
    },
    /// A component is non-finite or outside the normalized range.
    #[error("vertex {vertex} color component {component} must be finite and within 0..=1")]
    InvalidComponent {
        /// Vertex offset.
        vertex: usize,
        /// RGBA component offset.
        component: usize,
    },
}

impl Mesh3d {
    /// Linear, straight-alpha RGBA multipliers, or `None` for implicit white.
    pub fn vertex_colors(&self) -> Option<&[[f32; 4]]> {
        self.vertex_colors.as_deref()
    }

    /// Attaches normalized linear RGBA data without changing geometry or tangents.
    /// Each component must be finite and within 0..=1. Snapshots retain their colors;
    /// unchanged geometry and coordinate storage remain shared.
    pub fn with_vertex_colors(
        &self,
        colors: Vec<[f32; 4]>,
    ) -> Result<Arc<Self>, VertexColorError3d> {
        if colors.len() != self.vertices.len() {
            return Err(VertexColorError3d::Count {
                expected: self.vertices.len(),
                actual: colors.len(),
            });
        }
        for (vertex, color) in colors.iter().enumerate() {
            if let Some(component) = color.iter().position(|v| !(0. ..=1.).contains(v)) {
                return Err(VertexColorError3d::InvalidComponent { vertex, component });
            }
        }
        Ok(Arc::new(Self {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
            tangents: self.tangents.clone(),
            tangent_uv_set: self.tangent_uv_set,
            uv_sets: self.uv_sets.clone(),
            vertex_colors: Some(colors.into()),
            bounds: self.bounds,
        }))
    }
}
