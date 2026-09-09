mod constraints;
mod graph;
pub use constraints::TransformConstraint;
pub use graph::{
    EvaluatedNode, EvaluatedScene, Node, NodeHandle, ReparentMode, SceneError, SceneGraph,
    SceneSubtree, SubtreeInstance, SubtreeNode,
};

use crate::{
    AffineTransform, Camera, ColorOutput, DiffuseEnvironment, DirectionalShadow, Light, Material,
    Mesh, ObjectId, PickBehavior, PunctualLight, Ray, Transform, math, spatial::bvh,
};
use std::{
    borrow::Cow,
    sync::{Arc, OnceLock},
};

/// One mesh with a material and object-to-world transform.
#[derive(Clone)]
pub struct Object {
    pub(crate) cast_shadows: bool,
    pub(crate) receive_shadows: bool,
    pub(crate) id: Option<ObjectId>,
    pub(crate) pick_behavior: PickBehavior,
    pub(crate) mesh: Mesh,
    pub(crate) material: Material,
    pub(crate) transform: Transform,
    pub(crate) node: Option<NodeHandle>,
    pub(crate) world: Option<AffineTransform>,
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
            cast_shadows: true,
            receive_shadows: true,
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
    /// Casts opaque or alpha-masked shadows. Defaults to true; Blend never casts.
    pub fn cast_shadows(mut self, enabled: bool) -> Self {
        self.cast_shadows = enabled;
        self
    }
    /// Receives directional shadows when lit. Defaults to true.
    pub fn receive_shadows(mut self, enabled: bool) -> Self {
        self.receive_shadows = enabled;
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

/// Camera, lighting and objects for one independent depth buffer.
#[derive(Clone, Default)]
pub struct Scene {
    pub(crate) preparation_revision: Arc<()>,
    pub(crate) camera: Camera,
    pub(crate) light: Light,
    pub(crate) lights: Option<Arc<[gpui::PunctualLight3d]>>,
    pub(crate) directional_shadow: Option<DirectionalShadow>,
    pub(crate) diffuse_environment: Option<DiffuseEnvironment>,
    pub(crate) background: Option<crate::EnvironmentBackground>,
    pub(crate) specular_environment: Option<crate::SpecularEnvironment>,
    pub(crate) color_output: ColorOutput,
    pub(crate) objects: Vec<Object>,
    pub(crate) spatial_index: Arc<OnceLock<bvh::ObjectIndex>>,
    pub(crate) spatial_source: Option<Arc<Vec<bvh::IndexObject>>>,
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
    /// Sets a single directional light and ambient illumination, clearing explicit lights.
    pub fn light(mut self, light: Light) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.light = light;
        self.lights = None;
        self
    }
    /// Replaces direct lighting with at most MAX_PUNCTUAL_LIGHTS world-space sources.
    /// An empty list disables direct light. Ambient and environment illumination are preserved.
    /// Rendering rejects excess lights rather than truncating them.
    pub fn lights(mut self, lights: impl IntoIterator<Item = PunctualLight>) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.lights = Some(lights.into_iter().map(|light| light.0).collect());
        self
    }
    /// Sets one directional shadow map. None disables shadows; invalid settings fail rendering.
    pub fn directional_shadow(mut self, shadow: Option<DirectionalShadow>) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.directional_shadow = shadow;
        self
    }
    /// Adds distant diffuse illumination without changing the scene background.
    pub fn diffuse_environment(mut self, environment: DiffuseEnvironment) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.diffuse_environment = Some(environment);
        self
    }
    /// Displays a distant HDR environment independently of scene lighting and picking.
    /// None preserves transparent background; zero intensity draws opaque black.
    pub fn background(mut self, background: Option<crate::EnvironmentBackground>) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.background = background;
        self
    }
    /// Sets distant PBR reflections independently of background and diffuse illumination.
    pub fn specular_environment(mut self, environment: Option<crate::SpecularEnvironment>) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.specular_environment = environment;
        self
    }
    /// Sets exposure and tone mapping for the scene's linear HDR result.
    pub fn color_output(mut self, output: ColorOutput) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.color_output = output;
        self
    }
    /// Adds an object; distinct opaque depths do not depend on insertion order.
    pub fn object(mut self, object: Object) -> Self {
        Arc::make_mut(&mut self.preparation_revision);
        self.objects.push(object);
        self.spatial_source = None;
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
            .get_or_init(|| bvh::ObjectIndex::build_from(&self.index_source()));
    }

    /// Prepares this scene's object index using a previously prepared snapshot.
    /// Compatible object slots retain their partition; changed leaves and ancestors
    /// receive new bounds. Graph node identities survive ordering and visibility changes.
    /// Flat scenes use object positions as slots. Slot-set or boundedness changes rebuild
    /// the tree, as does an unprepared previous scene. Already prepared scenes are unchanged.
    /// Old snapshots remain queryable. This synchronous CPU operation needs no GPU.
    pub fn prepare_spatial_index_from(&self, previous: &Scene) {
        self.spatial_index.get_or_init(|| {
            let source = self.index_source();
            previous.spatial_index.get().map_or_else(
                || bvh::ObjectIndex::build_from(&source),
                |index| index.refit(&source),
            )
        });
    }

    fn index_source(&self) -> Cow<'_, [bvh::IndexObject]> {
        self.spatial_source.as_ref().map_or_else(
            || Cow::Owned(bvh::IndexObject::flat(&self.objects)),
            |source| Cow::Borrowed(source.as_slice()),
        )
    }

    pub(crate) fn visit_objects(&self, ray: Ray, visit: impl FnMut(usize)) {
        self.spatial_index
            .get_or_init(|| bvh::ObjectIndex::build_from(&self.index_source()))
            .visit(ray, visit);
    }
}
