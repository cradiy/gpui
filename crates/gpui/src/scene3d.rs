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
    vertices: Box<[MeshVertex3d]>,
    indices: Box<[u32]>,
}

impl Mesh3d {
    /// Creates triangles with counterclockwise front faces.
    /// Panics for empty, non-finite or out-of-range geometry.
    #[track_caller]
    pub fn new(vertices: Vec<MeshVertex3d>, indices: Vec<u32>) -> Arc<Self> {
        assert!(!vertices.is_empty() && !indices.is_empty() && indices.len().is_multiple_of(3));
        assert!(indices.iter().all(|i| (*i as usize) < vertices.len()));
        assert!(vertices.iter().all(|v| {
            v.position
                .iter()
                .chain(&v.normal)
                .chain(&v.uv)
                .all(|x| x.is_finite())
        }));
        Arc::new(Self {
            vertices: vertices.into(),
            indices: indices.into(),
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
}

/// Color input for an opaque or alpha-cutout mesh.
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

/// One indexed mesh and its material parameters.
#[derive(Clone, Debug)]
pub struct MeshDraw3d {
    /// Shared geometry.
    pub mesh: Arc<Mesh3d>,
    /// Column-major object-to-world matrix.
    pub model: [[f32; 4]; 4],
    /// Column-major inverse-transpose model matrix, applied to normals with W = 0.
    pub normal: [[f32; 4]; 4],
    /// Base tint, multiplied by the texture.
    pub color: Rgba,
    /// Material texture.
    pub texture: MeshTexture3d,
    /// Alpha below this threshold is discarded. Surviving pixels are opaque.
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
    /// Direction toward the light in world space.
    pub light_direction: [f32; 3],
    /// Light RGB and intensity multiplier.
    pub light: [f32; 4],
    /// Ambient light multiplier.
    pub ambient: f32,
    /// Meshes; opaque visibility is independent of submission order.
    pub objects: Arc<[MeshDraw3d]>,
}
