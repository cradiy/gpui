use anyhow::{Context, Result, bail, ensure};
use gltf::{
    Accessor, Semantic,
    accessor::{DataType, Dimensions},
    mesh::Mode,
};
use gpui_3d::{Mesh, NormalMode, SkinInfluence, Vertex};

use crate::PreparedDocument;

/// Per-primitive geometry conversion and element admission.
#[derive(Clone, Copy, Debug)]
pub struct GeometryOptions {
    /// UV set copied into the core mesh. A primitive without any UVs uses zero UVs.
    pub tex_coord_set: u32,
    /// Generate MikkTSpace tangents from the selected UVs, replacing authored tangents.
    pub generate_tangents: bool,
    /// Maximum input and final output vertices. Generation workspace is index-bounded.
    pub vertex_limit: usize,
    /// Maximum expanded triangle indices.
    pub index_limit: usize,
    /// Maximum input and final joint/weight slots, including zero weights.
    pub influence_limit: usize,
}

impl Default for GeometryOptions {
    fn default() -> Self {
        Self {
            tex_coord_set: 0,
            generate_tangents: false,
            vertex_limit: 4_194_304,
            index_limit: 12_582_912,
            influence_limit: 16_777_216,
        }
    }
}

/// Converted geometry with original glTF identities and vertex correspondence.
#[derive(Clone, Debug)]
pub struct PrimitiveGeometry {
    mesh: Mesh,
    source_vertices: Vec<u32>,
    mesh_index: usize,
    primitive_index: usize,
    material_index: Option<usize>,
    tex_coord_set: Option<u32>,
    pub(crate) influences: Option<crate::skin::VertexInfluences>,
}

impl PrimitiveGeometry {
    pub fn mesh(&self) -> &Mesh {
        &self.mesh
    }
    /// Source accessor element for each output vertex, including generated splits.
    pub fn source_vertices(&self) -> &[u32] {
        &self.source_vertices
    }
    pub fn mesh_index(&self) -> usize {
        self.mesh_index
    }
    pub fn primitive_index(&self) -> usize {
        self.primitive_index
    }
    pub fn material_index(&self) -> Option<usize> {
        self.material_index
    }
    /// None when the primitive has no texture coordinates.
    pub fn tex_coord_set(&self) -> Option<u32> {
        self.tex_coord_set
    }
    /// Joint indices address a skin's joint array, not document node indices.
    /// Each slice belongs to one output vertex, after normal/tangent splitting.
    pub fn skin_influences(&self) -> Option<std::slice::ChunksExact<'_, SkinInfluence>> {
        self.influences
            .as_ref()
            .map(|data| data.values.chunks_exact(data.stride))
    }
    pub fn influence_count(&self) -> usize {
        self.influences.as_ref().map_or(0, |data| data.values.len())
    }
    /// Returns base geometry and vertex correspondence, discarding skin inputs.
    pub fn into_parts(self) -> (Mesh, Vec<u32>) {
        (self.mesh, self.source_vertices)
    }
}

impl PreparedDocument {
    /// Converts one primitive without loading images, constructing nodes, or applying materials.
    /// Errors include the original mesh and primitive indices; prepared resources are unchanged.
    pub fn geometry(
        &self,
        mesh_index: usize,
        primitive_index: usize,
        options: GeometryOptions,
    ) -> Result<PrimitiveGeometry> {
        self.convert_geometry(mesh_index, primitive_index, options)
            .with_context(|| format!("mesh {mesh_index} primitive {primitive_index}"))
    }

    fn convert_geometry(
        &self,
        mesh_index: usize,
        primitive_index: usize,
        options: GeometryOptions,
    ) -> Result<PrimitiveGeometry> {
        crate::validation::supported_extensions(self.gltf())?;
        let source_mesh = self
            .gltf()
            .meshes()
            .nth(mesh_index)
            .context("mesh index out of range")?;
        let primitive = source_mesh
            .primitives()
            .nth(primitive_index)
            .context("primitive index out of range")?;
        ensure!(
            primitive.morph_targets().next().is_none(),
            "morph target conversion is unsupported"
        );
        let positions = primitive
            .get(&Semantic::Positions)
            .context("POSITION is required")?;
        let count = positions.count();
        ensure!(
            count <= options.vertex_limit && count <= u32::MAX as usize,
            "input vertex count {count} exceeds vertex limit"
        );
        let normals = primitive.get(&Semantic::Normals);
        let tangents = primitive.get(&Semantic::Tangents);
        let uv = primitive.get(&Semantic::TexCoords(options.tex_coord_set));
        let mut has_uvs = false;
        for (semantic, accessor) in primitive.attributes() {
            ensure!(
                accessor.count() == count,
                "{semantic:?} accessor {} count differs from POSITION",
                accessor.index()
            );
            let valid = match semantic {
                Semantic::Positions | Semantic::Normals => float(&accessor, Dimensions::Vec3),
                Semantic::Tangents => float(&accessor, Dimensions::Vec4),
                Semantic::TexCoords(_) => {
                    has_uvs = true;
                    accessor.dimensions() == Dimensions::Vec2
                        && match accessor.data_type() {
                            DataType::F32 => !accessor.normalized(),
                            DataType::U8 | DataType::U16 => accessor.normalized(),
                            _ => false,
                        }
                }
                Semantic::Joints(_) => {
                    accessor.dimensions() == Dimensions::Vec4
                        && !accessor.normalized()
                        && matches!(accessor.data_type(), DataType::U8 | DataType::U16)
                }
                Semantic::Weights(_) => {
                    accessor.dimensions() == Dimensions::Vec4
                        && match accessor.data_type() {
                            DataType::F32 => !accessor.normalized(),
                            DataType::U8 | DataType::U16 => accessor.normalized(),
                            _ => false,
                        }
                }
                _ => bail!("unsupported vertex attribute {semantic:?}"),
            };
            ensure!(
                valid,
                "unsupported {semantic:?} accessor {} format",
                accessor.index()
            );
        }
        ensure!(
            !has_uvs || uv.is_some(),
            "TEXCOORD_{} is unavailable",
            options.tex_coord_set
        );
        ensure!(
            !options.generate_tangents || uv.is_some(),
            "tangent generation requires texture coordinates"
        );

        let index_accessor = primitive.indices();
        let index_count = index_accessor.as_ref().map_or(count, Accessor::count);
        let expanded_count = match primitive.mode() {
            Mode::Triangles => {
                ensure!(
                    index_count > 0 && index_count.is_multiple_of(3),
                    "invalid triangle index count {index_count}"
                );
                index_count
            }
            Mode::TriangleStrip | Mode::TriangleFan => {
                ensure!(
                    index_count >= 3,
                    "triangle strip/fan requires at least three indices"
                );
                (index_count - 2)
                    .checked_mul(3)
                    .context("expanded index count overflow")?
            }
            mode => bail!("unsupported primitive mode {mode:?}"),
        };
        ensure!(
            expanded_count <= options.index_limit && expanded_count <= u32::MAX as usize,
            "expanded index count {expanded_count} exceeds index limit"
        );
        if let Some(accessor) = &index_accessor {
            ensure!(
                accessor.dimensions() == Dimensions::Scalar
                    && !accessor.normalized()
                    && matches!(
                        accessor.data_type(),
                        DataType::U8 | DataType::U16 | DataType::U32
                    ),
                "unsupported index accessor {} format",
                accessor.index()
            );
            ensure!(
                accessor.view().is_none_or(|view| view.stride().is_none()),
                "index accessor {} must not have a byte stride",
                accessor.index()
            );
        }

        let reader = primitive.reader(|buffer| self.buffer(buffer.index()));
        let sequence = if let Some(accessor) = &index_accessor {
            let indices = collect(accessor, reader.read_indices().map(|v| v.into_u32()))?;
            let restart = match accessor.data_type() {
                DataType::U8 => u8::MAX as u32,
                DataType::U16 => u16::MAX as u32,
                _ => u32::MAX,
            };
            for (offset, &index) in indices.iter().enumerate() {
                ensure!(
                    index != restart,
                    "index {offset} uses reserved primitive restart value"
                );
                ensure!(
                    (index as usize) < count,
                    "index {offset} references vertex {index} outside POSITION"
                );
            }
            indices
        } else {
            (0..count as u32).collect()
        };
        let indices = match primitive.mode() {
            Mode::Triangles => sequence,
            mode => {
                let mut indices = Vec::with_capacity(expanded_count);
                for i in 0..index_count - 2 {
                    let corners = if mode == Mode::TriangleFan {
                        [sequence[i + 1], sequence[i + 2], sequence[0]]
                    } else {
                        [
                            sequence[i],
                            sequence[i + 1 + i % 2],
                            sequence[i + 2 - i % 2],
                        ]
                    };
                    indices.extend(corners);
                }
                indices
            }
        };
        let position_values = collect(&positions, reader.read_positions())?;
        let normal_values = normals
            .as_ref()
            .map(|accessor| collect(accessor, reader.read_normals()))
            .transpose()?;
        let uv_values = uv
            .as_ref()
            .map(|accessor| {
                collect(
                    accessor,
                    reader
                        .read_tex_coords(options.tex_coord_set)
                        .map(|v| v.into_f32()),
                )
            })
            .transpose()?;
        let mut vertices = Vec::with_capacity(count);
        for (index, position) in position_values.into_iter().enumerate() {
            let normal = match &normal_values {
                Some(values) => {
                    let value = values[index].map(f64::from);
                    let length = value.iter().map(|v| v * v).sum::<f64>().sqrt();
                    ensure!(
                        length.is_finite() && length > 0.,
                        "invalid NORMAL at vertex {index}"
                    );
                    value.map(|v| (v / length) as f32)
                }
                None => [0.; 3],
            };
            vertices.push(Vertex {
                position,
                normal,
                uv: uv_values.as_ref().map_or([0.; 2], |values| values[index]),
            });
        }
        let mut mesh = Mesh::try_new(vertices, indices).context("mesh attributes")?;
        let mut source_vertices: Vec<u32> = (0..count as u32).collect();
        if normals.is_none() {
            (mesh, source_vertices) = mesh
                .generate_normals(NormalMode::Flat)
                .context("flat normal generation")?
                .into_parts();
        } else if !options.generate_tangents
            && let Some(accessor) = &tangents
        {
            mesh = mesh
                .with_tangents(collect(accessor, reader.read_tangents())?)
                .context("authored tangents")?;
        }
        if options.generate_tangents {
            let (generated, mapping) = mesh
                .generate_tangents()
                .context("tangent generation")?
                .into_parts();
            source_vertices = mapping
                .iter()
                .map(|&index| source_vertices[index as usize])
                .collect();
            mesh = generated;
        }
        ensure!(
            mesh.vertex_count() <= options.vertex_limit,
            "generated vertex count {} exceeds vertex limit",
            mesh.vertex_count()
        );
        let influences = crate::skin::influences(
            self,
            &primitive,
            count,
            &source_vertices,
            options.influence_limit,
        )?;
        Ok(PrimitiveGeometry {
            mesh,
            source_vertices,
            mesh_index,
            primitive_index,
            material_index: primitive.material().index(),
            tex_coord_set: uv.map(|_| options.tex_coord_set),
            influences,
        })
    }
}

fn float(accessor: &Accessor<'_>, dimensions: Dimensions) -> bool {
    accessor.data_type() == DataType::F32
        && accessor.dimensions() == dimensions
        && !accessor.normalized()
}

pub(crate) fn collect<T: Default + Clone>(
    accessor: &Accessor<'_>,
    values: Option<impl Iterator<Item = T>>,
) -> Result<Vec<T>> {
    if accessor.view().is_none() && accessor.sparse().is_none() {
        return Ok(vec![T::default(); accessor.count()]);
    }
    let values: Vec<_> = values
        .with_context(|| format!("accessor {} data is unavailable", accessor.index()))?
        .collect();
    ensure!(
        values.len() == accessor.count(),
        "accessor {} decoded count mismatch",
        accessor.index()
    );
    Ok(values)
}
