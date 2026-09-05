//! GPU-driven visual effects for GPUI applications.
//!
//! Effects use WGSL as their canonical implementation. Applications may add
//! native MSL and HLSL implementations through [`gpui::EffectShader`] when an
//! effect needs platform-specific tuning.
//!
//! See [`glass_guide`] for the mergeable frosted-glass material.
//! See [`liquid_glass_guide`] for refractive material configuration and painting.
//! See [`timed_text_guide`] for karaoke timelines, grouped emphasis, and
//! playback-clock integration.

/// Complete usage guide for [`FrostedGlass`](crate::FrostedGlass).
#[doc = include_str!("../docs/glass.md")]
pub mod glass_guide {}

/// Usage guide for [`LiquidGlass`](crate::LiquidGlass) and [`paint_liquid_glass`].
#[doc = include_str!("../docs/liquid_glass.md")]
pub mod liquid_glass_guide {}

/// Usage guide for [`TimedText`](crate::TimedText).
#[doc = include_str!("../docs/timed_text.md")]
pub mod timed_text_guide {}

mod backdrop;
mod builtins;
mod color_flow;
mod element;
mod flip;
mod glass;
mod liquid_glass;
mod masked_builtins;
mod masked_effect;
mod masked_fill;
mod motion;
mod sticky;
mod text_blur;
mod timed_text;

pub use backdrop::*;
pub use builtins::*;
pub use color_flow::{
    ColorFlow, ColorFlowOptions, ColorFlowPalette, ColorFlowPaletteColor, color_flow,
    color_flow_shader,
};
pub use element::{Effect, effect, four_image_effect, image_effect, two_image_effect};
pub use flip::{
    FLIP_APPEARANCE_SLOT, FLIP_BACKGROUND_SLOT, FLIP_INTERACTION_SLOT, FLIP_LAYOUT_SLOT,
    FLIP_REGIONS_SLOT, Flip, FlipDirection, FlipEntry, FlipEvent, FlipImageRegion, FlipJumpResult,
    FlipLayout, FlipObjectFit, FlipPositionReason, FlipPreloadReason, FlipReadingDirection,
    FlipRequestResult, FlipSlot, FlipStyle, FlipUpdateResult, flip_shader, flip_shader_for,
    rigid_flip_shader, soft_flip_shader,
};
pub use glass::{FrostedGlass, FrostedGlassAppearance, FrostedGlassShape};
pub use liquid_glass::{LiquidGlass, LiquidGlassAppearance, liquid_glass_shader, paint_liquid_glass};
pub use masked_builtins::{spectrum_mask_shader, spectrum_svg, spectrum_text};
pub use masked_effect::{MaskedEffect, effect_svg, effect_text, masked_effect};
pub use masked_fill::{MaskedFill, gradient_svg, gradient_text, masked_fill};
pub use motion::{
    MotionEasing, MotionEvent, MotionFrame, MotionId, MotionItem, MotionLayer, MotionOptions,
    MotionPath, MotionPolicy,
};
pub use sticky::{StickyShape, paint_sticky_shapes, sticky_shape_shader};
pub use text_blur::TextBlur;
pub use timed_text::{TimedText, TimedTextEmphasis, TimedTextRevealWave, TimedTextUnit};
