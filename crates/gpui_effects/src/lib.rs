//! GPU-driven visual effects for GPUI applications.
//!
//! Effects use WGSL as their canonical implementation. Applications may add
//! native MSL and HLSL implementations through [`gpui::EffectShader`] when an
//! effect needs platform-specific tuning.

mod builtins;
mod element;
mod flip;
mod masked_builtins;
mod masked_effect;
mod masked_fill;
mod motion;

pub use builtins::*;
pub use element::{Effect, effect, four_image_effect, image_effect, two_image_effect};
pub use flip::{
    FLIP_APPEARANCE_SLOT, FLIP_BACKGROUND_SLOT, FLIP_INTERACTION_SLOT, FLIP_LAYOUT_SLOT,
    FLIP_REGIONS_SLOT, Flip, FlipDirection, FlipEntry, FlipEvent, FlipImageRegion, FlipJumpResult,
    FlipLayout, FlipObjectFit, FlipPositionReason, FlipPreloadReason, FlipReadingDirection,
    FlipRequestResult, FlipSlot, FlipStyle, FlipUpdateResult, flip_shader, flip_shader_for,
    rigid_flip_shader, soft_flip_shader,
};
pub use masked_builtins::{spectrum_mask_shader, spectrum_svg, spectrum_text};
pub use masked_effect::{MaskedEffect, effect_svg, effect_text, masked_effect};
pub use masked_fill::{MaskedFill, gradient_svg, gradient_text, masked_fill};
pub use motion::{
    MotionEasing, MotionEvent, MotionFrame, MotionId, MotionItem, MotionLayer, MotionOptions,
    MotionPath, MotionPolicy,
};
