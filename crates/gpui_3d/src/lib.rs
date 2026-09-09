//! Depth-tested mesh viewports embedded in GPUI layouts.

/// Camera, material, layout and rendering guide.
#[doc = include_str!("../docs/viewport.md")]
pub mod guide {}

mod camera;
mod geometry;
mod lighting;
mod material;
mod math;
mod render;
mod scene;
mod spatial;

pub use camera::{
    Camera, CameraError, OrbitController, OrbitError, OrbitSettings, Projection, Ray, RayError,
    ScreenPoint,
};
pub use geometry::{Aabb, AffineTransform, Mesh, Transform, TransformError};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use headless::*;
pub use lighting::{
    DiffuseEnvironment, DirectionalShadow, EnvironmentBackground, EnvironmentError, EnvironmentMap,
    Light, PunctualLight,
};
pub use material::{Material, MaterialTexture};
pub(crate) use material::{Texture, TextureSlot};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use render::headless;
pub use render::{Viewport3d, viewport3d};
pub use scene::{
    EvaluatedNode, EvaluatedScene, Node, NodeHandle, Object, ReparentMode, Scene, SceneError,
    SceneGraph, SceneSubtree, SubtreeInstance, SubtreeNode,
};
pub use spatial::{Hit, PickBehavior};

pub use gpui::AlphaMode3d as AlphaMode;
pub use gpui::ElementId as ObjectId;
pub use gpui::MAX_PUNCTUAL_LIGHTS_3D as MAX_PUNCTUAL_LIGHTS;
pub use gpui::MeshVertex3d as Vertex;
pub use gpui::PbrMaterial3d as PbrMaterial;
pub use gpui::TangentError3d as TangentError;
pub use gpui::{
    ColorOutput3d as ColorOutput, TextureColorSpace3d as TextureColorSpace,
    ToneMapping3d as ToneMapping,
};
pub use gpui::{MeshError3d as MeshError, MeshVertexAttribute3d as VertexAttribute};
pub use gpui::{
    TextureAddressMode3d as TextureAddressMode, TextureFilter3d as TextureFilter,
    TextureSampling3d as TextureSampling, UvTransform3d as UvTransform,
    UvTransformError3d as UvTransformError,
};
