//! Depth-tested mesh viewports embedded in GPUI layouts.

mod animation;
mod camera;
mod geometry;
mod lighting;
mod material;
mod math;
mod render;
mod scene;
mod spatial;

pub use animation::{
    AnimationError, Interpolation, Keyframe, Pose, PoseError, PoseMask, RotationTrack,
    TransformPose, TransformTrack, VectorTrack, WeightPose, WeightPoseError, WeightTrack,
};
pub use camera::{
    Camera, CameraError, Frustum, OrbitController, OrbitError, OrbitSettings, Projection, Ray,
    RayError, ScreenPoint,
};
pub use geometry::{
    Aabb, AffineTransform, AimError, AimResult, AimSettings, AimStatus, ConeOptions,
    CylinderOptions, GeneratedNormals, GeneratedTangents, IkChainError, IkChainJoint,
    IkChainResult, IkChainSettings, IkChainStatus, IkOrientationStatus, IkOrientationTarget,
    IkReach, JointAngleLimitStatus, JointRotationLimitError, JointRotationLimitResult,
    JointRotationLimits, Mesh, MorphAttribute, MorphError, MorphTarget, MorphTargets,
    NormalGenerationError, NormalMode, NormalizedSkinInfluence, PlaneOptions, PrimitiveError, Skin,
    SkinError, SkinInfluence, SphereOptions, TangentGenerationError, TangentGenerationMode,
    TangentRepair, TangentRepairKind, Transform, TransformError, TwoBoneIkError, TwoBoneIkResult,
    TwoBoneIkSettings,
};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use geometry::{
    GpuDeformationBounds, GpuDeformationBoundsReadback, GpuDeformationLimits, GpuDeformationOutput,
    GpuDeformationReadback, GpuDeformationVertex, GpuFlatNormals, GpuFlatNormalsMemory,
    GpuGeometryPreparation, GpuMorph, GpuMorphMemory, GpuSkin, GpuSkinMemory, GpuSkinPalette,
    GpuTangentAdjacency, GpuTangentAdjacencyMemory, GpuTangentAdjacencyOutput,
    GpuTangentDerivative, GpuTangentDerivativeOutput, GpuTangentDerivatives,
    GpuTangentDerivativesMemory, GpuTangentEdge, GpuTangentFrame, GpuTangentFrames,
    GpuTangentFramesMemory, GpuTangentFramesOutput, GpuTangentGroup, GpuTangentGroups,
    GpuTangentGroupsMemory, GpuTangentGroupsOutput, GpuTangentWeld, GpuTangentWeldMemory,
    GpuTangentWeldOutput, GpuTangentWeldRecord, GpuTangents, GpuTangentsMemory, GpuTangentsOutput,
    PreparedGpuGeometry,
};
pub use gpui::MeshTexture3d as ResolvedTexture;
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use gpui_wgpu::{
    Scene3dGeometryIssues, Scene3dGeometryStatus, Scene3dGeometryStatusReadback,
    Scene3dMaterialBindingLimits, Scene3dMaterialLimits, Scene3dMaterialProgram,
    Scene3dMaterialResource, Scene3dMaterialResourceKind, Scene3dMaterialSnapshot,
    Scene3dMaterialSource, Scene3dMaterialValue, Scene3dVertexAttribute,
    Scene3dVertexInterpolation, Scene3dVertexStreamValue, Scene3dVertexStreams,
    Scene3dVertexUpdate, WgpuResource,
};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use headless::*;
pub use lighting::{
    DiffuseEnvironment, DirectionalShadow, EnvironmentBackground, EnvironmentError, EnvironmentMap,
    Light, LightError, PunctualLight, SpecularEnvironment, SpecularEnvironmentMap,
    SpecularPrefilter,
};
pub(crate) use material::Texture;
pub use material::{
    Material, MaterialTexture, MeshPass, MeshPassBlend, MeshPassCull, MeshPassDepth,
    MeshPassExpansion, MeshPassSpace, MeshPassStage, MeshPassState, TextureSlot,
};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use render::headless;
pub use render::{
    GeometryInput, PendingTexture, PreparationCache, PrepareError, PreparedScene, RenderObject,
    TextureRequest, TextureSource, TextureState, Viewport3d, viewport3d,
};
#[cfg(all(feature = "wgpu", not(target_family = "wasm")))]
pub use render::{ViewportPick, ViewportPickCapture, ViewportPickFrame, ViewportPickReadback};
pub use scene::{
    ConstraintStatus, EvaluatedNode, EvaluatedScene, Node, NodeHandle, Object, ObjectUpdate,
    ReparentMode, Scene, SceneError, SceneGraph, SceneSubtree, SubtreeInstance, SubtreeNode,
    TransformConstraint, TransformOverride,
};
pub use spatial::{Hit, PickBehavior, QueryObject};

pub use gpui::AlphaMode3d as AlphaMode;
pub use gpui::DepthBackground3d as DepthBackground;
pub use gpui::ElementId as ObjectId;
pub use gpui::LightKind3d as LightKind;
pub use gpui::MAX_PUNCTUAL_LIGHTS_3D as MAX_PUNCTUAL_LIGHTS;
pub use gpui::MeshUpdateError3d as MeshUpdateError;
pub use gpui::MeshVertex3d as Vertex;
pub use gpui::PbrMaterial3d as PbrMaterial;
pub use gpui::Scene3dViewportQuality as ViewportQuality;
pub use gpui::TangentError3d as TangentError;
pub use gpui::UvSetError3d as UvSetError;
pub use gpui::VertexColorError3d as VertexColorError;
pub use gpui::{
    ColorOutput3d as ColorOutput, TextureColorSpace3d as TextureColorSpace,
    ToneMapping3d as ToneMapping,
};
pub use gpui::{MeshError3d as MeshError, MeshVertexAttribute3d as VertexAttribute};
pub use gpui::{
    TextureAddressMode3d as TextureAddressMode, TextureFilter3d as TextureFilter,
    TextureMipFilter3d as TextureMipFilter, TextureSampling3d as TextureSampling,
    UvTransform3d as UvTransform, UvTransformError3d as UvTransformError,
};
