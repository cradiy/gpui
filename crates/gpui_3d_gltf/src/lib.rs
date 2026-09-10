//! glTF 2.0 scenes, materials, and animation tracks with caller-owned resource loading.
//!
//! Parsing and preparation perform no filesystem, network, image decoding, or GPU
//! operations. URI policy, loading, retries, and decoded-image budgets belong to
//! the caller. Prepared resources preserve glTF indices for scene conversion.

mod animation;
mod cache;
mod camera;
mod geometry;
mod image;
mod image_cache;
mod instance;
mod light;
mod load_queue;
mod loading;
mod material;
mod morph;
mod playback;
mod resources;
mod scene;
mod skin;
mod validation;

pub use animation::{AnimationClip, AnimationOptions, NodeAnimation};
pub use cache::{ResourceCache, ResourceCacheLimits};
pub use geometry::{GeometryOptions, PrimitiveGeometry};
pub use image::{DecodedScene, ImageDecodeLimits};
pub use image_cache::{ImageCache, ImageCacheLimits};
pub use instance::SceneInstance;
pub use load_queue::{SceneLoadQueue, SceneLoadQueueFull, SceneLoadQueueStats};
pub use loading::{SceneLoadCompletion, SceneLoadRequest, SceneLoadSlot, SceneLoadStatus};
pub use material::{MaterialDefinition, TextureBinding};
pub use morph::{MorphGeometry, SceneMorph};
pub use playback::AnimationPlayback;
pub use resources::{Document, EncodedImage, Limits, PreparedDocument, ResourceRequest};
pub use scene::{SceneAsset, SceneDefinition, SceneNode, SceneOptions, ScenePrimitive};
pub use skin::{SceneSkin, SkinDefinition, SkinOptions};
