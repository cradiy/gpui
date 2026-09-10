//! glTF 2.0 scenes, materials, and animation tracks with caller-owned resource loading.
//!
//! Parsing and preparation perform no filesystem, network, image decoding, or GPU
//! operations. URI policy, loading, retries, and decoded-image budgets belong to
//! the caller. Prepared resources preserve glTF indices for scene conversion.

/// Resource admission, scene conversion, image decoding, and animation sampling.
#[doc = include_str!("../docs/resources.md")]
#[doc = include_str!("../docs/geometry.md")]
#[doc = include_str!("../docs/materials.md")]
#[doc = include_str!("../docs/scenes.md")]
#[doc = include_str!("../docs/images.md")]
#[doc = include_str!("../docs/animation.md")]
pub mod guide {}

mod animation;
mod camera;
mod geometry;
mod image;
mod material;
mod resources;
mod scene;
mod validation;

pub use animation::{AnimationClip, AnimationOptions, NodeAnimation};
pub use geometry::{GeometryOptions, PrimitiveGeometry};
pub use image::ImageDecodeLimits;
pub use material::{MaterialDefinition, TextureBinding};
pub use resources::{Document, EncodedImage, Limits, PreparedDocument};
pub use scene::{SceneAsset, SceneDefinition, SceneNode, SceneOptions, ScenePrimitive};
