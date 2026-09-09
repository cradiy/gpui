//! Depth-tested mesh viewports embedded in GPUI layouts.

/// Camera, material, layout and rendering guide.
#[doc = include_str!("../docs/viewport.md")]
pub mod guide {}

mod math;
mod viewport;

pub use gpui::MeshVertex3d as Vertex;
use gpui::{ImageSource, Mesh3d, Rgba};
pub use math::{Camera, Transform};
use std::sync::{Arc, OnceLock};
pub use viewport::{Viewport3d, viewport3d};

/// Shared immutable indexed geometry.
#[derive(Clone, Debug)]
pub struct Mesh(Arc<Mesh3d>);
impl Mesh {
    /// Creates counterclockwise triangles from mesh-local vertex data.
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        Self(Mesh3d::new(vertices, indices))
    }
    /// Unit XY plane centered at the origin, facing positive Z.
    pub fn plane() -> Self {
        static PLANE: OnceLock<Arc<Mesh3d>> = OnceLock::new();
        Self(
            PLANE
                .get_or_init(|| face_mesh(&[([0., 0., 0.], [1., 0., 0.], [0., 1., 0.])]))
                .clone(),
        )
    }
    /// Unit cube centered at the origin, with per-face normals and UVs.
    pub fn cube() -> Self {
        static CUBE: OnceLock<Arc<Mesh3d>> = OnceLock::new();
        Self(
            CUBE.get_or_init(|| {
                face_mesh(&[
                    ([0., 0., 0.5], [1., 0., 0.], [0., 1., 0.]),
                    ([0., 0., -0.5], [-1., 0., 0.], [0., 1., 0.]),
                    ([0.5, 0., 0.], [0., 0., -1.], [0., 1., 0.]),
                    ([-0.5, 0., 0.], [0., 0., 1.], [0., 1., 0.]),
                    ([0., 0.5, 0.], [1., 0., 0.], [0., 0., -1.]),
                    ([0., -0.5, 0.], [1., 0., 0.], [0., 0., 1.]),
                ])
            })
            .clone(),
        )
    }
}

type Face = ([f32; 3], [f32; 3], [f32; 3]);
fn face_mesh(faces: &[Face]) -> Arc<Mesh3d> {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
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
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| i + first));
    }
    Mesh3d::new(vertices, indices)
}

#[derive(Clone)]
enum Texture {
    None,
    Image(ImageSource),
    Ui,
}

/// Solid or textured material with optional alpha cutout.
#[derive(Clone)]
pub struct Material {
    color: Rgba,
    texture: Texture,
    unlit: bool,
    alpha_cutoff: f32,
}
impl Material {
    /// Creates a lit solid material.
    pub fn color(color: impl Into<Rgba>) -> Self {
        Self {
            color: color.into(),
            texture: Texture::None,
            unlit: false,
            alpha_cutoff: 0.5,
        }
    }
    /// Uses an image's first decoded frame, stretched over mesh UVs.
    pub fn image(image: impl Into<ImageSource>) -> Self {
        Self {
            texture: Texture::Image(image.into()),
            ..Self::color(gpui::white())
        }
    }
    /// Uses the viewport's decorative UI capture without lighting.
    pub fn ui() -> Self {
        Self {
            texture: Texture::Ui,
            unlit: true,
            ..Self::color(gpui::white())
        }
    }
    /// Bypasses directional and ambient lighting.
    pub fn unlit(mut self, unlit: bool) -> Self {
        self.unlit = unlit;
        self
    }
    /// Discards source alpha below the threshold. Remaining pixels are opaque.
    pub fn alpha_cutoff(mut self, cutoff: f32) -> Self {
        assert!(cutoff.is_finite());
        self.alpha_cutoff = cutoff.clamp(0.001, 1.);
        self
    }
}

/// One mesh with a material and object-to-world transform.
#[derive(Clone)]
pub struct Object {
    mesh: Mesh,
    material: Material,
    transform: Transform,
}
impl Object {
    /// Creates a mesh at the origin.
    pub fn new(mesh: Mesh, material: Material) -> Self {
        Self {
            mesh,
            material,
            transform: Transform::default(),
        }
    }
    /// Sets world position.
    pub fn position(mut self, position: [f32; 3]) -> Self {
        self.transform.position = position;
        self
    }
    /// Sets XYZ Euler angles in radians.
    pub fn rotation(mut self, rotation: [f32; 3]) -> Self {
        self.transform.rotation = rotation;
        self
    }
    /// Sets nonzero per-axis scale.
    pub fn scale(mut self, scale: [f32; 3]) -> Self {
        self.transform.scale = scale;
        self
    }
    /// Replaces the full object transform.
    pub fn transform(mut self, transform: Transform) -> Self {
        self.transform = transform;
        self
    }
}

/// One directional light plus uniform ambient illumination.
#[derive(Clone, Copy, Debug)]
pub struct Light {
    /// Direction toward the light in world space.
    pub direction: [f32; 3],
    /// Light color; alpha is ignored.
    pub color: Rgba,
    /// Direct light multiplier.
    pub intensity: f32,
    /// Ambient light multiplier.
    pub ambient: f32,
}
impl Default for Light {
    fn default() -> Self {
        Self {
            direction: [-0.5, 0.8, 0.7],
            color: gpui::rgb(0xe7efff),
            intensity: 0.75,
            ambient: 0.3,
        }
    }
}

/// Camera, lighting and objects for one independent depth buffer.
#[derive(Clone, Default)]
pub struct Scene {
    camera: Camera,
    light: Light,
    objects: Vec<Object>,
}
impl Scene {
    /// Creates an empty scene with the default camera and light.
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets the perspective camera.
    pub fn camera(mut self, camera: Camera) -> Self {
        self.camera = camera;
        self
    }
    /// Sets scene lighting.
    pub fn light(mut self, light: Light) -> Self {
        self.light = light;
        self
    }
    /// Adds an object; distinct opaque depths do not depend on insertion order.
    pub fn object(mut self, object: Object) -> Self {
        self.objects.push(object);
        self
    }
}
