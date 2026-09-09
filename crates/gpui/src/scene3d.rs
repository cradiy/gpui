use std::sync::Arc;

use crate::{AtlasTile, DevicePixels, Pixels, Rgba, Size, size};

/// Capabilities of the current window's depth-tested mesh path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scene3dViewportCapabilities {
    /// Maximum physical dimension of a render target or image texture.
    pub max_texture_dimension: u32,
    /// Maximum color sample count selected by the renderer (one or four).
    pub color_samples: u32,
    /// Maximum physical dimension of a captured UI texture.
    pub max_ui_texture_dimension: u32,
}

impl Scene3dViewportCapabilities {
    /// Effective samples for a viewport, falling back to one when four are unavailable.
    pub fn color_samples_for(self, quality: Scene3dViewportQuality) -> u32 {
        if self.color_samples == 4 && quality.color_samples == 4 {
            4
        } else {
            1
        }
    }
}

/// Mesh raster density and color sampling, independent of UI layout and capture density.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scene3dViewportQuality {
    resolution_scale: f32,
    color_samples: u32,
}

impl Default for Scene3dViewportQuality {
    fn default() -> Self {
        Self {
            resolution_scale: 1.,
            color_samples: 4,
        }
    }
}

impl Scene3dViewportQuality {
    /// Creates viewport quality settings. Scale must be finite and positive;
    /// samples must be one or four. Raster dimensions are capped by the device.
    #[track_caller]
    pub fn new(resolution_scale: f32, color_samples: u32) -> Self {
        assert!(
            resolution_scale.is_finite() && resolution_scale > 0.,
            "resolution scale must be finite and positive"
        );
        assert!(
            matches!(color_samples, 1 | 4),
            "color samples must be one or four"
        );
        Self {
            resolution_scale,
            color_samples,
        }
    }

    /// Requested mesh pixel density relative to the render surface's physical pixels.
    pub fn resolution_scale(self) -> f32 {
        self.resolution_scale
    }

    /// Requested color samples. Use the current capabilities to resolve a fallback.
    pub fn color_samples(self) -> u32 {
        self.color_samples
    }
}

/// Why a window cannot currently render mesh viewports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scene3dUnsupportedReason {
    /// The platform renderer has no mesh viewport implementation.
    BackendUnsupported,
    /// Renderer resources are temporarily absent, for example during recovery.
    RendererUnavailable,
    /// The renderer has observed loss of its graphics device.
    DeviceLost,
    /// A required device limit or format feature is unavailable.
    MissingCapabilities(crate::SharedString),
}
impl std::fmt::Display for Scene3dUnsupportedReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BackendUnsupported => {
                f.write_str("the window backend does not implement 3D viewports")
            }
            Self::RendererUnavailable => f.write_str("the window renderer is unavailable"),
            Self::DeviceLost => {
                f.write_str("the graphics device is lost; renderer recovery is required")
            }
            Self::MissingCapabilities(reason) => reason.fmt(f),
        }
    }
}
impl std::error::Error for Scene3dUnsupportedReason {}

/// Current window support, independent of direct headless output capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scene3dSupport {
    /// Mesh viewports can use the reported capabilities.
    Supported(Scene3dViewportCapabilities),
    /// Mesh viewports are unavailable for the given reason.
    Unsupported(Scene3dUnsupportedReason),
}
impl Scene3dSupport {
    /// Whether mesh viewports are currently available.
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported(_))
    }
    /// The current mesh capabilities, or `None` when unavailable.
    pub fn capabilities(&self) -> Option<Scene3dViewportCapabilities> {
        match self {
            Self::Supported(capabilities) => Some(*capabilities),
            Self::Unsupported(_) => None,
        }
    }
}

mod environment;
mod visibility;
pub use environment::{
    EnvironmentBackground3d, EnvironmentError3d, EnvironmentMap3d, SpecularEnvironment3d,
    SpecularEnvironmentMap3d,
};

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
    bounds: [[f32; 3]; 2],
}

/// Invalid fixed-topology vertex replacement.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MeshUpdateError3d {
    /// Replacement vertices must preserve every vertex, including unused vertices.
    #[error("expected {expected} vertices, received {actual}")]
    VertexCount {
        /// Source vertex count.
        expected: usize,
        /// Supplied vertex count.
        actual: usize,
    },
    /// Invalid replacement attributes.
    #[error(transparent)]
    Geometry(#[from] MeshError3d),
    /// Invalid replacement tangent basis.
    #[error(transparent)]
    Tangents(#[from] TangentError3d),
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
        Self::validate_vertices(&vertices)?;
        Ok(Arc::new(Self {
            bounds: visibility::bounds(&vertices, &indices),
            vertices: vertices.into(),
            indices: indices.into(),
            tangents: None,
        }))
    }

    fn validate_vertices(vertices: &[MeshVertex3d]) -> Result<(), MeshError3d> {
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
        Ok(())
    }

    /// Replaces fixed-count vertex attributes while sharing triangle index storage.
    /// The source and its snapshots remain unchanged. Tangents must be supplied
    /// for the new normals, or `None` explicitly removes them.
    pub fn with_vertices(
        &self,
        vertices: Vec<MeshVertex3d>,
        tangents: Option<Vec<[f32; 4]>>,
    ) -> Result<Arc<Self>, MeshUpdateError3d> {
        if vertices.len() != self.vertices.len() {
            return Err(MeshUpdateError3d::VertexCount {
                expected: self.vertices.len(),
                actual: vertices.len(),
            });
        }
        Self::validate_vertices(&vertices)?;
        let mesh = Self {
            bounds: visibility::bounds(&vertices, &self.indices),
            vertices: vertices.into(),
            indices: self.indices.clone(),
            tangents: None,
        };
        Ok(match tangents {
            Some(tangents) => mesh.with_tangents(tangents)?,
            None => Arc::new(mesh),
        })
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
            bounds: self.bounds,
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

/// Maximum number of explicit direct lights per scene.
pub const MAX_PUNCTUAL_LIGHTS_3D: usize = 8;

/// Direct-light spatial distribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind3d {
    /// Infinite source with a constant direction toward the light.
    Directional,
    /// Inverse-square source emitting in every direction.
    Point,
    /// Inverse-square source emitting along a cone.
    Spot,
}

/// World-space direct light. No shadows or scene-node attachment are implied.
#[derive(Clone, Copy, Debug)]
pub struct PunctualLight3d {
    /// Spatial distribution.
    pub kind: LightKind3d,
    /// World-space position; ignored by directional lights.
    pub position: [f32; 3],
    /// Toward a directional source, or outward from a spot source. Nonzero.
    pub direction: [f32; 3],
    /// sRGB color in [0, 1]; alpha is ignored.
    pub color: crate::Rgba,
    /// Nonnegative linear multiplier, at most 65504.
    pub intensity: f32,
    /// Optional positive distance cutoff; ignored by directional lights.
    pub range: Option<f32>,
    /// Inverse-square denominator clamp in world units, in [0.0001, 65504].
    pub minimum_distance: f32,
    /// Spot half-angle with full intensity, in radians.
    pub inner_angle: f32,
    /// Spot half-angle with zero intensity; inner < outer <= pi/2.
    pub outer_angle: f32,
}

impl PunctualLight3d {
    /// Checks finite values, supported ranges, directions, and cone angles.
    pub fn is_valid(&self) -> bool {
        self.position
            .iter()
            .chain(&self.direction)
            .all(|v| v.is_finite())
            && (self.kind == LightKind3d::Point || self.direction.iter().any(|v| *v != 0.))
            && [self.color.r, self.color.g, self.color.b]
                .iter()
                .all(|v| v.is_finite() && (0. ..=1.).contains(v))
            && self.intensity.is_finite()
            && (0. ..=65504.).contains(&self.intensity)
            && self.range.is_none_or(|v| v.is_finite() && v > 0.)
            && self.minimum_distance.is_finite()
            && (0.0001..=65504.).contains(&self.minimum_distance)
            && self.inner_angle.is_finite()
            && self.inner_angle >= 0.
            && self.outer_angle.is_finite()
            && self.inner_angle < self.outer_angle
            && self.outer_angle <= std::f32::consts::FRAC_PI_2
            && self.inner_angle.cos() > self.outer_angle.cos()
    }
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

/// Shadow-map projection and sampling for one directional source.
#[derive(Clone, Copy, Debug)]
pub struct DirectionalShadow3d {
    /// Index into explicit lights, or zero for the single directional light.
    pub light_index: u32,
    /// Column-major world-to-light clip transform; XY in [-1, 1], depth in [0, 1].
    pub view_projection: [[f32; 4]; 4],
    /// Power-of-two side length from 256 through 4096.
    pub resolution: u32,
    /// Receiver depth offset in normalized light depth, in [0, 0.05].
    pub depth_bias: f32,
    /// Receiver offset along the geometric normal, in nonnegative world units.
    pub normal_bias: f32,
    /// PCF radius in shadow texels, in [0, 4]. Zero selects a hard comparison.
    pub softness: f32,
}
impl DirectionalShadow3d {
    /// Checks finite projection/sampling parameters and supported map dimensions.
    pub fn is_valid(self) -> bool {
        self.view_projection.iter().flatten().all(|v| v.is_finite())
            && self.resolution.is_power_of_two()
            && (256..=4096).contains(&self.resolution)
            && self.depth_bias.is_finite()
            && (0. ..=0.05).contains(&self.depth_bias)
            && self.normal_bias.is_finite()
            && self.normal_bias >= 0.
            && self.softness.is_finite()
            && (0. ..=4.).contains(&self.softness)
    }
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
    /// Cast opaque or alpha-masked shadows. Blend materials never cast shadows.
    pub cast_shadows: bool,
    /// Receive direct-light shadows; ignored by unlit materials.
    pub receive_shadows: bool,
}

/// Immutable input for a depth-tested 3D viewport.
#[derive(Clone, Debug)]
pub struct Scene3dFrame {
    /// Window viewport raster quality. Direct renderers use their output configuration instead.
    pub viewport_quality: Scene3dViewportQuality,
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
    /// Explicit direct lights replacing the single light when present. At most eight.
    pub lights: Option<Arc<[PunctualLight3d]>>,
    /// Optional shadow map for one directional source.
    pub directional_shadow: Option<DirectionalShadow3d>,
    /// Ambient light multiplier.
    pub ambient: f32,
    /// Optional distant diffuse illumination. It does not draw a background.
    pub diffuse_environment: Option<DiffuseEnvironment3d>,
    /// Distant color background; does not contribute to lighting or geometry outputs.
    pub background: Option<EnvironmentBackground3d>,
    /// Optional roughness-dependent distant specular illumination for PBR materials.
    pub specular_environment: Option<SpecularEnvironment3d>,
    /// HDR-to-display conversion. Does not affect depth or object IDs.
    pub color_output: crate::ColorOutput3d,
    /// Meshes; opaque visibility is independent of submission order.
    pub objects: Arc<[MeshDraw3d]>,
}

impl Scene3dFrame {
    /// Checks shadow settings and that their selected source is directional.
    pub fn shadow_is_valid(&self) -> bool {
        self.directional_shadow.is_none_or(|shadow| {
            shadow.is_valid()
                && self.lights.as_ref().map_or_else(
                    || {
                        shadow.light_index == 0
                            && self.light_direction.iter().all(|v| v.is_finite())
                            && self.light_direction.iter().any(|v| *v != 0.)
                    },
                    |lights| {
                        lights
                            .get(shadow.light_index as usize)
                            .is_some_and(|light| {
                                light.kind == LightKind3d::Directional && light.is_valid()
                            })
                    },
                )
        })
    }
}
