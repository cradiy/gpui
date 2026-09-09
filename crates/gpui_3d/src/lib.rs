//! Depth-tested mesh viewports embedded in GPUI layouts.

/// Camera, material, layout and rendering guide.
#[doc = include_str!("../docs/viewport.md")]
pub mod guide {}

mod affine;
mod bounds;
mod bvh;
mod camera;
mod environment;
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
pub use environment::{DiffuseEnvironment, EnvironmentError};
pub use gpui::AlphaMode3d as AlphaMode;
pub use gpui::ElementId as ObjectId;
pub use gpui::MeshVertex3d as Vertex;
pub use gpui::PbrMaterial3d as PbrMaterial;
pub use gpui::TangentError3d as TangentError;
pub use gpui::{
    ColorOutput3d as ColorOutput, TextureColorSpace3d as TextureColorSpace,
    ToneMapping3d as ToneMapping,
};
use gpui::{ImageSource, Mesh3d, Rgba};
pub use gpui::{MeshError3d as MeshError, MeshVertexAttribute3d as VertexAttribute};
pub use gpui::{
    TextureAddressMode3d as TextureAddressMode, TextureFilter3d as TextureFilter,
    TextureSampling3d as TextureSampling, UvTransform3d as UvTransform,
    UvTransformError3d as UvTransformError,
};
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
    /// Mesh-local tangent XYZ and handedness W, if supplied.
    pub fn tangents(&self) -> Option<&[[f32; 4]]> {
        self.0.tangents()
    }
    /// Attaches validated tangents without changing geometry, triangle identities
    /// or the source mesh. XYZ is normalized and orthogonalized against normals;
    /// W must be -1 or +1, constant within each triangle.
    pub fn with_tangents(&self, tangents: Vec<[f32; 4]>) -> Result<Self, TangentError> {
        self.0
            .with_tangents(tangents)
            .map(|mesh| Self(mesh, self.1.clone()))
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
    let mut tangents = Vec::new();
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
            tangents.push([right[0], right[1], right[2], -1.]);
        }
        indices.extend([0, 1, 2, 0, 2, 3].map(|i| i + first));
    }
    Mesh3d::new(vertices, indices)
        .with_tangents(tangents)
        .expect("invalid face tangents")
}

#[derive(Clone)]
enum Texture {
    None,
    Image(ImageSource),
    Ui,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextureSlot {
    BaseColor,
    MetallicRoughness,
    Emissive,
    Normal,
    Occlusion,
}

/// An image and its independent mesh-UV sampling configuration.
#[derive(Clone)]
pub struct MaterialTexture {
    image: ImageSource,
    sampling: TextureSampling,
}

impl MaterialTexture {
    /// Uses the first decoded frame with linear filtering and clamped coordinates.
    pub fn new(image: impl Into<ImageSource>) -> Self {
        Self {
            image: image.into(),
            sampling: TextureSampling::default(),
        }
    }

    /// Sets the UV transform, per-axis addressing and mip-zero filtering.
    pub fn sampling(mut self, sampling: TextureSampling) -> Self {
        self.sampling = sampling;
        self
    }
}

/// Solid or textured material with explicit alpha interpretation.
#[derive(Clone)]
pub struct Material {
    color: Rgba,
    texture: Texture,
    unlit: bool,
    alpha_cutoff: f32,
    alpha_mode: AlphaMode,
    sampling: TextureSampling,
    image_color_space: TextureColorSpace,
    pbr: Option<PbrMaterial>,
    metallic_roughness_texture: Option<MaterialTexture>,
    emissive_texture: Option<MaterialTexture>,
    normal_texture: Option<MaterialTexture>,
    normal_scale: f32,
    occlusion_texture: Option<MaterialTexture>,
    occlusion_strength: f32,
}
impl Material {
    /// Creates a lit solid material from an sRGB color.
    pub fn color(color: impl Into<Rgba>) -> Self {
        Self {
            color: color.into(),
            texture: Texture::None,
            unlit: false,
            alpha_cutoff: 0.5,
            alpha_mode: AlphaMode::Mask,
            sampling: TextureSampling::default(),
            image_color_space: TextureColorSpace::default(),
            pbr: None,
            metallic_roughness_texture: None,
            emissive_texture: None,
            normal_texture: None,
            normal_scale: 1.,
            occlusion_texture: None,
            occlusion_strength: 1.,
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
    /// Sets an sRGB tint, decoded before multiplication; alpha follows the alpha mode.
    pub fn tint(mut self, color: impl Into<Rgba>) -> Self {
        self.color = color.into();
        self
    }
    /// Configures image textures. Captured UI textures retain their original
    /// UV mapping and linear edge-clamped sampling.
    pub fn image_sampling(mut self, sampling: TextureSampling) -> Self {
        self.sampling = sampling;
        self
    }
    /// Sets image RGB encoding. Alpha remains linear; solid tint and UI captures
    /// use sRGB regardless of this setting.
    pub fn image_color_space(mut self, color_space: TextureColorSpace) -> Self {
        self.image_color_space = color_space;
        self
    }
    /// Enables metallic-roughness shading, preserving the base color, texture and cutoff.
    /// Rendering rejects invalid parameters. Unlit mode bypasses PBR, including emission.
    pub fn pbr(mut self, parameters: PbrMaterial) -> Self {
        self.pbr = Some(parameters);
        self
    }
    /// Multiplies PBR roughness by linear G and metallic by linear B; R and alpha
    /// are ignored. Only used when PBR is enabled and the material is lit.
    pub fn metallic_roughness_texture(mut self, texture: MaterialTexture) -> Self {
        self.metallic_roughness_texture = Some(texture);
        self
    }
    /// Multiplies PBR emission by sRGB RGB decoded before filtering. Alpha is
    /// ignored. Only used when PBR is enabled and the material is lit.
    pub fn emissive_texture(mut self, texture: MaterialTexture) -> Self {
        self.emissive_texture = Some(texture);
        self
    }
    /// Uses linear RGB tangent-space normals decoded from [0, 1] to [-1, 1].
    /// Alpha is ignored. Requires mesh tangents when lit PBR and nonzero scale
    /// are enabled. Does not change geometry, silhouettes or picking normals.
    pub fn normal_texture(mut self, texture: MaterialTexture) -> Self {
        self.normal_texture = Some(texture);
        self
    }
    /// Scales normal-map XY before normalization. Defaults to 1; zero disables
    /// the map and its resource requests. Rendering rejects negative/non-finite values.
    pub fn normal_scale(mut self, scale: f32) -> Self {
        self.normal_scale = scale;
        self
    }

    /// Attenuates diffuse ambient and environment light using linear R.
    /// G, B and alpha are ignored. Works with basic and PBR lit materials.
    pub fn occlusion_texture(mut self, texture: MaterialTexture) -> Self {
        self.occlusion_texture = Some(texture);
        self
    }
    /// Blends from no occlusion at 0 to the full map at 1. Defaults to 1.
    /// Zero disables resource requests. Rendering rejects values outside [0, 1].
    pub fn occlusion_strength(mut self, strength: f32) -> Self {
        self.occlusion_strength = strength;
        self
    }

    pub(crate) fn lighting_textures(
        &self,
    ) -> impl Iterator<Item = (TextureSlot, &MaterialTexture)> {
        [
            (
                TextureSlot::MetallicRoughness,
                self.metallic_roughness_texture.as_ref(),
            ),
            (TextureSlot::Emissive, self.emissive_texture.as_ref()),
            (
                TextureSlot::Normal,
                self.normal_texture
                    .as_ref()
                    .filter(|_| self.normal_scale != 0.),
            ),
            (
                TextureSlot::Occlusion,
                self.occlusion_texture
                    .as_ref()
                    .filter(|_| self.occlusion_strength != 0.),
            ),
        ]
        .into_iter()
        .filter_map(|(slot, texture)| {
            texture
                .filter(|_| !self.unlit && (slot == TextureSlot::Occlusion || self.pbr.is_some()))
                .map(|texture| (slot, texture))
        })
    }
    /// Bypasses lighting, occlusion, and emission.
    pub fn unlit(mut self, unlit: bool) -> Self {
        self.unlit = unlit;
        self
    }
    /// Selects Opaque, Mask or Blend alpha interpretation. Defaults to Mask.
    pub fn alpha_mode(mut self, mode: AlphaMode) -> Self {
        self.alpha_mode = mode;
        self
    }
    /// Sets the Mask threshold in [0.001, 1] and selects Mask mode.
    pub fn alpha_cutoff(mut self, cutoff: f32) -> Self {
        assert!(cutoff.is_finite());
        self.alpha_cutoff = cutoff.clamp(0.001, 1.);
        self.alpha_mode = AlphaMode::Mask;
        self
    }

    fn alpha_visible(&self, alpha: f32) -> bool {
        let alpha = alpha.clamp(0., 1.);
        match self.alpha_mode {
            AlphaMode::Opaque => true,
            AlphaMode::Mask => alpha >= self.alpha_cutoff,
            AlphaMode::Blend => alpha > 0.,
        }
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
    /// sRGB light color; alpha is ignored.
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
    diffuse_environment: Option<DiffuseEnvironment>,
    color_output: ColorOutput,
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
    /// Adds distant diffuse illumination without changing the scene background.
    pub fn diffuse_environment(mut self, environment: DiffuseEnvironment) -> Self {
        self.diffuse_environment = Some(environment);
        self
    }
    /// Sets exposure and tone mapping for the scene's linear HDR result.
    pub fn color_output(mut self, output: ColorOutput) -> Self {
        self.color_output = output;
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
