//! glTF 2.0 scenes, materials, and animation tracks with caller-owned resource loading.
//!
//! Parsing and preparation perform no filesystem, network, image decoding, or GPU
//! operations. URI policy, loading, retries, and decoded-image budgets belong to
//! the caller. Prepared resources preserve glTF indices for scene conversion.

mod animation;
mod camera;
mod geometry;
mod image;
mod material;
mod morph;
mod resources;
mod scene;
mod skin;
mod validation;

pub use animation::{AnimationClip, AnimationOptions, NodeAnimation};
pub use geometry::{GeometryOptions, PrimitiveGeometry};
pub use image::ImageDecodeLimits;
pub use material::{MaterialDefinition, TextureBinding};
pub use morph::{MorphGeometry, SceneMorph};
pub use resources::{Document, EncodedImage, Limits, PreparedDocument};
pub use scene::{SceneAsset, SceneDefinition, SceneNode, SceneOptions, ScenePrimitive};
pub use skin::{SceneSkin, SkinDefinition, SkinOptions};
