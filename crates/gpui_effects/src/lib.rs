//! GPU-driven visual effects for GPUI applications.
//!
//! Effects use WGSL as their canonical implementation. Applications may add
//! native MSL and HLSL implementations through [`gpui::EffectShader`] when an
//! effect needs platform-specific tuning.

mod builtins;
mod element;

pub use builtins::*;
pub use element::{Effect, effect, four_image_effect, image_effect, two_image_effect};
