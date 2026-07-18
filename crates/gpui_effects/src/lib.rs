//! GPU-driven visual effects for GPUI applications.
//!
//! Effects use WGSL as their canonical implementation. Applications may add
//! native MSL and HLSL implementations through [`gpui::EffectShader`] when an
//! effect needs platform-specific tuning.

mod builtins;
mod element;
mod masked_fill;

pub use builtins::*;
pub use element::{Effect, effect, four_image_effect, image_effect, two_image_effect};
pub use masked_fill::{MaskedFill, gradient_svg, gradient_text, masked_fill};
