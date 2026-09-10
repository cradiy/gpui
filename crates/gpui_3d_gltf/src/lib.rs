//! glTF 2.0 resources, meshes, and materials with caller-owned URI resolution and decoding.
//!
//! Parsing and preparation perform no filesystem, network, image decoding, or GPU
//! operations. URI policy, loading, retries, and decoded-image budgets belong to
//! the caller. Prepared resources preserve glTF indices for scene conversion.

/// Resource admission, URI resolution, mesh conversion, and material preparation.
#[doc = include_str!("../docs/resources.md")]
#[doc = include_str!("../docs/geometry.md")]
#[doc = include_str!("../docs/materials.md")]
pub mod guide {}

mod geometry;
mod material;
mod resources;
mod validation;

pub use geometry::{GeometryOptions, PrimitiveGeometry};
pub use material::{MaterialDefinition, TextureBinding};
pub use resources::{Document, EncodedImage, Limits, PreparedDocument};
