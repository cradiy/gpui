//! glTF 2.0 documents and resource preparation with caller-owned URI resolution.
//!
//! Parsing and preparation perform no filesystem, network, image decoding, or GPU
//! operations. URI policy, loading, retries, and decoded-image budgets belong to
//! the caller. Prepared resources preserve glTF indices for scene conversion.

/// Resource admission, URI resolution, and binary-layout contracts.
#[doc = include_str!("../docs/resources.md")]
pub mod guide {}

mod resources;
mod validation;

pub use resources::{Document, EncodedImage, Limits, PreparedDocument};
