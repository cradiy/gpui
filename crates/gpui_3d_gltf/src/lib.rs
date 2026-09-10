//! glTF 2.0 resource preparation and mesh conversion with caller-owned URI resolution.
//!
//! Parsing and preparation perform no filesystem, network, image decoding, or GPU
//! operations. URI policy, loading, retries, and decoded-image budgets belong to
//! the caller. Prepared resources preserve glTF indices for scene conversion.

/// Resource admission, URI resolution, and primitive conversion.
#[doc = include_str!("../docs/resources.md")]
#[doc = include_str!("../docs/geometry.md")]
pub mod guide {}

mod geometry;
mod resources;
mod validation;

pub use geometry::{GeometryOptions, PrimitiveGeometry};
pub use resources::{Document, EncodedImage, Limits, PreparedDocument};
