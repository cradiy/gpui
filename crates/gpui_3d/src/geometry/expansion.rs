use crate::{Mesh, MeshExpansionError};

/// An unshared triangle-corner mesh and its output-to-source vertex mapping.
#[derive(Clone, Debug)]
pub struct ExpandedMesh {
    mesh: Mesh,
    source_vertices: Vec<u32>,
}

impl ExpandedMesh {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }

    /// Source index for every output vertex, in original triangle-corner order.
    /// Use this mapping for external attributes, Morph deltas and Skin influences.
    pub fn source_vertices(&self) -> &[u32] {
        &self.source_vertices
    }

    pub fn into_parts(self) -> (Mesh, Vec<u32>) {
        (self.mesh, self.source_vertices)
    }
}

impl Mesh {
    /// Creates one vertex per indexed triangle corner without regenerating directions.
    /// All stored attributes and triangle order are preserved; unused vertices are
    /// omitted. The output has sequential indices and an independent spatial index.
    /// The vertex limit is checked before allocating output storage.
    pub fn expand_corners(&self, vertex_limit: usize) -> Result<ExpandedMesh, MeshExpansionError> {
        let mesh = self.0.expand_corners(vertex_limit)?;
        Ok(ExpandedMesh {
            mesh: Self(mesh, Default::default()),
            source_vertices: self.indices().to_vec(),
        })
    }
}
