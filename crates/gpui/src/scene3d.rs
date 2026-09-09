use std::sync::Arc;

use crate::{AtlasTile, DevicePixels, Pixels, Rgba, Size, size};

/// Logical dimensions and raster density of a decorative UI texture.
#[derive(Clone, Copy, Debug)]
pub struct UiTexture3d {
    logical_size: Size<Pixels>,
    scale_factor: f32,
}

impl UiTexture3d {
    /// Creates a texture with a uniform number of device pixels per logical pixel.
    /// Density is reduced uniformly when either dimension would exceed 2048 pixels.
    #[track_caller]
    pub fn new(logical_size: Size<Pixels>, scale_factor: f32) -> Self {
        let width = f32::from(logical_size.width);
        let height = f32::from(logical_size.height);
        assert!(width.is_finite() && width > 0. && height.is_finite() && height > 0.);
        assert!(scale_factor.is_finite() && scale_factor > 0.);
        Self {
            logical_size,
            scale_factor: scale_factor.min(2048. / width.max(height)),
        }
    }

    /// Layout dimensions, independent of the window and mesh dimensions.
    pub fn logical_size(self) -> Size<Pixels> {
        self.logical_size
    }

    /// Effective raster density after the texture size limit is applied.
    pub fn scale_factor(self) -> f32 {
        self.scale_factor
    }

    /// Allocated pixel dimensions, rounded up to cover the full layout.
    pub fn pixel_size(self) -> Size<DevicePixels> {
        size(
            DevicePixels(
                (f32::from(self.logical_size.width) * self.scale_factor)
                    .ceil()
                    .clamp(1., 2048.) as i32,
            ),
            DevicePixels(
                (f32::from(self.logical_size.height) * self.scale_factor)
                    .ceil()
                    .clamp(1., 2048.) as i32,
            ),
        )
    }
}

/// A position, normal and texture coordinate in mesh-local space.
#[derive(Clone, Copy, Debug)]
pub struct MeshVertex3d {
    /// Right-handed XYZ coordinates.
    pub position: [f32; 3],
    /// Surface normal; nonzero normals are normalized during shading.
    pub normal: [f32; 3],
    /// Texture coordinates, with (0, 0) at the top left.
    pub uv: [f32; 2],
}

/// Immutable indexed triangles. Share the same allocation to reuse GPU buffers.
#[derive(Debug)]
pub struct Mesh3d {
    vertices: Arc<[MeshVertex3d]>,
    indices: Arc<[u32]>,
    tangents: Option<Arc<[[f32; 4]]>>,
}

/// Invalid tangent data. Offsets are zero-based.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TangentError3d {
    /// Every vertex, including unused vertices, requires a tangent.
    #[error("expected {expected} tangents, received {actual}")]
    Count {
        /// Mesh vertex count.
        expected: usize,
        /// Supplied tangent count.
        actual: usize,
    },
    /// The tangent or normal cannot form a finite basis, or W is not -1 or +1.
    #[error("invalid tangent basis at vertex {vertex}")]
    InvalidBasis {
        /// Invalid vertex offset.
        vertex: usize,
    },
    /// Mirrored UV seams must use separate vertices.
    #[error("mixed tangent handedness in triangle {triangle}")]
    MixedHandedness {
        /// Triangle offset in the index buffer.
        triangle: usize,
    },
}

/// Vertex attribute containing an invalid component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshVertexAttribute3d {
    /// Mesh-local XYZ coordinates.
    Position,
    /// Mesh-local surface normal.
    Normal,
    /// Texture coordinates.
    Uv,
}

/// Invalid indexed triangle data. Vertex, index-buffer and component offsets
/// are zero-based.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeshError3d {
    /// No vertex data was supplied.
    #[error("mesh vertices are empty")]
    EmptyVertices,
    /// No triangle indices were supplied.
    #[error("mesh indices are empty")]
    EmptyIndices,
    /// The index buffer ends with a partial triangle.
    #[error("index count {index_count} is not a multiple of three")]
    IncompleteTriangle {
        /// Total number of supplied indices.
        index_count: usize,
    },
    /// An index does not reference a supplied vertex.
    #[error("index {index} at offset {offset} is outside {vertex_count} vertices")]
    IndexOutOfBounds {
        /// Position in the index buffer.
        offset: usize,
        /// Invalid vertex index.
        index: u32,
        /// Number of supplied vertices.
        vertex_count: usize,
    },
    /// A vertex attribute contains NaN or infinity.
    #[error("vertex {vertex} has a non-finite {attribute:?} component at {component}")]
    NonFiniteVertex {
        /// Position in the vertex buffer.
        vertex: usize,
        /// Attribute containing the invalid value.
        attribute: MeshVertexAttribute3d,
        /// Position within the attribute's components.
        component: usize,
    },
}

impl Mesh3d {
    /// Creates triangles with counterclockwise front faces.
    /// Panics for empty, non-finite or out-of-range geometry.
    #[track_caller]
    pub fn new(vertices: Vec<MeshVertex3d>, indices: Vec<u32>) -> Arc<Self> {
        Self::try_new(vertices, indices).expect("invalid mesh geometry")
    }

    /// Validates indexed triangles without panicking for invalid geometry.
    /// Retains vertex/index order, unused vertices and degenerate triangles.
    /// Zero normals and finite UVs outside 0..=1 are accepted.
    pub fn try_new(
        vertices: Vec<MeshVertex3d>,
        indices: Vec<u32>,
    ) -> Result<Arc<Self>, MeshError3d> {
        if vertices.is_empty() {
            return Err(MeshError3d::EmptyVertices);
        }
        if indices.is_empty() {
            return Err(MeshError3d::EmptyIndices);
        }
        if !indices.len().is_multiple_of(3) {
            return Err(MeshError3d::IncompleteTriangle {
                index_count: indices.len(),
            });
        }
        for (offset, &index) in indices.iter().enumerate() {
            if index as usize >= vertices.len() {
                return Err(MeshError3d::IndexOutOfBounds {
                    offset,
                    index,
                    vertex_count: vertices.len(),
                });
            }
        }
        for (vertex, data) in vertices.iter().enumerate() {
            for (attribute, values) in [
                (MeshVertexAttribute3d::Position, data.position.as_slice()),
                (MeshVertexAttribute3d::Normal, data.normal.as_slice()),
                (MeshVertexAttribute3d::Uv, data.uv.as_slice()),
            ] {
                if let Some(component) = values.iter().position(|value| !value.is_finite()) {
                    return Err(MeshError3d::NonFiniteVertex {
                        vertex,
                        attribute,
                        component,
                    });
                }
            }
        }
        Ok(Arc::new(Self {
            vertices: vertices.into(),
            indices: indices.into(),
            tangents: None,
        }))
    }

    /// Mesh-local vertex data.
    pub fn vertices(&self) -> &[MeshVertex3d] {
        &self.vertices
    }
    /// Triangle indices.
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Mesh-local tangent XYZ and handedness W. Bitangent is `cross(N, T) * W`.
    pub fn tangents(&self) -> Option<&[[f32; 4]]> {
        self.tangents.as_deref()
    }

    /// Creates a mesh sharing vertex/index storage with validated tangent data.
    /// XYZ is orthogonalized against the vertex normal and normalized. W must be
    /// -1 or +1 and constant within each triangle. The source mesh is unchanged.
    pub fn with_tangents(&self, mut tangents: Vec<[f32; 4]>) -> Result<Arc<Self>, TangentError3d> {
        if tangents.len() != self.vertices.len() {
            return Err(TangentError3d::Count {
                expected: self.vertices.len(),
                actual: tangents.len(),
            });
        }
        for (vertex, (tangent, data)) in tangents.iter_mut().zip(self.vertices.iter()).enumerate() {
            let invalid = TangentError3d::InvalidBasis { vertex };
            if !tangent.iter().all(|v| v.is_finite()) || tangent[3].abs() != 1. {
                return Err(invalid);
            }
            let n = data.normal.map(f64::from);
            let n_length = n.iter().map(|v| v * v).sum::<f64>().sqrt();
            if n_length == 0. {
                return Err(invalid);
            }
            let n = n.map(|v| v / n_length);
            let t = [tangent[0], tangent[1], tangent[2]].map(f64::from);
            let t_length = t.iter().map(|v| v * v).sum::<f64>().sqrt();
            let dot = t.iter().zip(n).map(|(t, n)| t * n).sum::<f64>();
            let t: [f64; 3] = std::array::from_fn(|i| t[i] - n[i] * dot);
            let length = t.iter().map(|v| v * v).sum::<f64>().sqrt();
            if length <= t_length * 1e-6 {
                return Err(invalid);
            }
            for i in 0..3 {
                tangent[i] = (t[i] / length) as f32;
            }
        }
        for (triangle, indices) in self.indices.chunks_exact(3).enumerate() {
            let sign = tangents[indices[0] as usize][3];
            if indices.iter().any(|i| tangents[*i as usize][3] != sign) {
                return Err(TangentError3d::MixedHandedness { triangle });
            }
        }
        Ok(Arc::new(Self {
            vertices: self.vertices.clone(),
            indices: self.indices.clone(),
            tangents: Some(tangents.into()),
        }))
    }
}

/// Interpretation of base-color alpha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum AlphaMode3d {
    /// Ignore alpha and write opaque color and depth.
    Opaque = 0,
    /// Discard pixels below the cutoff and make survivors opaque.
    #[default]
    Mask = 1,
    /// Premultiplied source-over blending, with depth testing but no depth writes.
    Blend = 2,
}

/// L2 real spherical harmonics of diffuse irradiance divided by pi, in linear RGB.
/// Coefficient order is (0,0), (1,-1), (1,0), (1,1), (2,-2), (2,-1), (2,0), (2,1), (2,2).
#[derive(Clone, Copy, Debug)]
pub struct DiffuseEnvironment3d {
    /// Convolved coefficients; each component is finite and in [-262016, 262016].
    pub coefficients: [[f32; 3]; 9],
    /// Nonnegative linear multiplier, at most 65504.
    pub intensity: f32,
    /// Environment-to-world rotation around +Y, in radians.
    pub rotation_y: f32,
}

impl DiffuseEnvironment3d {
    /// Whether all coefficients and controls fit the supported finite ranges.
    pub fn is_valid(&self) -> bool {
        self.coefficients
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() <= 4. * 65504.)
            && self.intensity.is_finite()
            && (0. ..=65504.).contains(&self.intensity)
            && self.rotation_y.is_finite()
    }
}

/// Color input for a mesh material.
#[derive(Clone, Copy, Debug, Default)]
pub enum MeshTexture3d {
    /// Solid material color.
    #[default]
    None,
    /// Straight-alpha image in the renderer's atlas.
    Image(AtlasTile),
    /// Premultiplied pixels from the viewport's captured UI subtree.
    Subtree,
}

/// A material image in the renderer atlas with independent mesh-UV sampling.
#[derive(Clone, Copy, Debug)]
pub struct MaterialTexture3d {
    /// Straight-alpha atlas image; material maps ignore alpha.
    pub tile: AtlasTile,
    /// Image-coordinate transform, addressing and filtering.
    pub sampling: crate::TextureSampling3d,
}

/// One indexed mesh and its material parameters.
#[derive(Clone, Debug)]
pub struct MeshDraw3d {
    /// Exact frame-local output ID. Zero is reserved for the background.
    /// The caller retains the mapping to application or scene-node identities.
    pub output_id: u32,
    /// Shared geometry.
    pub mesh: Arc<Mesh3d>,
    /// Column-major object-to-world matrix.
    pub model: [[f32; 4]; 4],
    /// Column-major inverse-transpose model matrix, applied to normals with W = 0.
    pub normal: [[f32; 4]; 4],
    /// sRGB base tint, decoded and multiplied by the linear texture color.
    pub color: Rgba,
    /// Material texture.
    pub texture: MeshTexture3d,
    /// Image sampling; solid materials and captured UI textures ignore this.
    pub sampling: crate::TextureSampling3d,
    /// Image RGB transfer function; does not affect solid colors or captured UI.
    pub image_color_space: crate::TextureColorSpace3d,
    /// Optional metallic-roughness shading. Unlit materials ignore these parameters.
    pub pbr: Option<crate::PbrMaterial3d>,
    /// Linear G roughness and B metallic multipliers. Ignored without lit PBR.
    pub metallic_roughness_texture: Option<MaterialTexture3d>,
    /// sRGB RGB emission multiplier, decoded before filtering. Ignored without lit PBR.
    pub emissive_texture: Option<MaterialTexture3d>,
    /// Linear tangent-space RGB normal map. Requires mesh tangents; lit PBR only.
    pub normal_texture: Option<MaterialTexture3d>,
    /// Finite nonnegative scale of normal-map XY. Zero disables the map.
    pub normal_scale: f32,
    /// Linear R attenuation of indirect light. Ignored for unlit materials.
    pub occlusion_texture: Option<MaterialTexture3d>,
    /// Finite occlusion blend in [0, 1]. Zero disables the map.
    pub occlusion_strength: f32,
    /// Base-color alpha interpretation.
    pub alpha_mode: AlphaMode3d,
    /// Finite camera-space forward depth for back-to-front Blend sorting.
    /// Equal-depth objects retain submission order. Ignored by other modes.
    pub sort_depth: f64,
    /// Mask alpha threshold; ignored in Opaque and Blend modes.
    pub alpha_cutoff: f32,
    /// Bypass directional lighting.
    pub unlit: bool,
}

/// Immutable input for a depth-tested 3D viewport.
#[derive(Clone, Debug)]
pub struct Scene3dFrame {
    /// An origin-zero UI capture with independent dimensions and density.
    /// `None` samples the viewport rectangle from the ordinary subtree target.
    pub ui_texture: Option<UiTexture3d>,
    /// Column-major world-to-clip matrix, with depth in 0 through 1.
    pub view_projection: [[f32; 4]; 4],
    /// Column-major world-to-view affine transform. Camera forward is local -Z;
    /// its negated Z coordinate is linear depth in scene units.
    pub world_to_view: [[f32; 4]; 4],
    /// World-space camera eye for perspective view-dependent shading.
    pub camera_position: [f32; 3],
    /// Constant world-space direction toward an orthographic viewer; `None` uses the eye.
    pub orthographic_view_direction: Option<[f32; 3]>,
    /// Direction toward the light in world space.
    pub light_direction: [f32; 3],
    /// sRGB light RGB and linear intensity multiplier.
    pub light: [f32; 4],
    /// Ambient light multiplier.
    pub ambient: f32,
    /// Optional distant diffuse illumination. It does not draw a background.
    pub diffuse_environment: Option<DiffuseEnvironment3d>,
    /// HDR-to-display conversion. Does not affect depth or object IDs.
    pub color_output: crate::ColorOutput3d,
    /// Meshes; opaque visibility is independent of submission order.
    pub objects: Arc<[MeshDraw3d]>,
}
