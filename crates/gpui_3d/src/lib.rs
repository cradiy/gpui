//! Depth-tested mesh viewports embedded in GPUI layouts.

/// Camera, material, layout and rendering guide.
#[doc = include_str!("../docs/viewport.md")]
pub mod guide {}

mod affine;
mod bounds;
mod bvh;
mod camera;
mod frame;
mod graph;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub mod headless;
mod math;
mod orbit;
mod picking;
mod ui_input;
mod viewport;

pub use affine::{AffineTransform, TransformError};
pub use bounds::Aabb;
pub use camera::{Camera, CameraError, Projection, Ray, RayError, ScreenPoint};
pub use gpui::ElementId as ObjectId;
pub use gpui::MeshVertex3d as Vertex;
use gpui::{ImageSource, Mesh3d, Rgba};
pub use gpui::{MeshError3d as MeshError, MeshVertexAttribute3d as VertexAttribute};
pub use graph::{
    EvaluatedNode, EvaluatedScene, Node, NodeHandle, ReparentMode, SceneError, SceneGraph,
    SceneSubtree, SubtreeInstance, SubtreeNode,
};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use headless::*;
pub use math::Transform;
pub use orbit::{OrbitController, OrbitError, OrbitSettings};
pub use picking::{Hit, PickBehavior};
use std::sync::{Arc, OnceLock};
pub use viewport::{Viewport3d, viewport3d};

/// Shared immutable indexed geometry.
#[derive(Clone, Debug)]
pub struct Mesh(Arc<Mesh3d>, Arc<OnceLock<bvh::Bvh>>);
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
    /// Sets the color multiplier for sampled RGBA, including alpha cutout.
    pub fn tint(mut self, color: impl Into<Rgba>) -> Self {
        self.color = color.into();
        self
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
    id: Option<ObjectId>,
    pick_behavior: PickBehavior,
    mesh: Mesh,
    material: Material,
    transform: Transform,
    node: Option<NodeHandle>,
    world: Option<AffineTransform>,
}
impl Object {
    pub(crate) fn matrices(&self) -> (math::Matrix, math::Matrix) {
        self.world.map_or_else(
            || self.transform.matrices(),
            |world| (world.matrix(), world.normal_matrix()),
        )
    }

    /// Creates a mesh at the origin.
    pub fn new(mesh: Mesh, material: Material) -> Self {
        Self {
            id: None,
            pick_behavior: PickBehavior::default(),
            mesh,
            material,
            transform: Transform::default(),
            node: None,
            world: None,
        }
    }
    /// Assigns a stable application-defined identity for picking callbacks.
    pub fn id(mut self, id: impl Into<ObjectId>) -> Self {
        self.id = Some(id.into());
        self
    }
    /// Controls picking without changing rendering or depth writes.
    pub fn pick_behavior(mut self, behavior: PickBehavior) -> Self {
        self.pick_behavior = behavior;
        self
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
    spatial_index: Arc<OnceLock<bvh::ObjectIndex>>,
}
impl Scene {
    /// Creates an empty scene with the default camera and light.
    pub fn new() -> Self {
        Self::default()
    }
    /// Sets the viewport camera.
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
        if let Some(index) = Arc::get_mut(&mut self.spatial_index) {
            index.take();
        } else {
            self.spatial_index = Arc::default();
        }
        self
    }

    /// Prepares the shared object index. Mesh triangle indices remain lazy.
    /// This synchronous CPU operation needs no window or GPU.
    pub fn prepare_spatial_index(&self) {
        self.spatial_index
            .get_or_init(|| bvh::ObjectIndex::build(&self.objects));
    }

    fn visit_objects(&self, ray: Ray, visit: impl FnMut(usize)) {
        self.spatial_index
            .get_or_init(|| bvh::ObjectIndex::build(&self.objects))
            .visit(ray, visit);
    }
}
