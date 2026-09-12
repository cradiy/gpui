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
mod bloom;
mod builtins;
mod color_flow;
mod contour_glow;
mod contour_relief;
mod contour_shadow;
mod deformation;
mod depth_parallax;
mod displacement_map;
mod effect_stage;
mod element;
mod feedback;
mod flip;
mod fluid;
mod glass;
mod lens;
mod liquid_glass;
mod masked_builtins;
mod masked_effect;
mod masked_fill;
mod material;
mod motion;
mod motion_blur;
mod particle_transition;
mod particles;
mod progressive_blur;
mod ripple;
mod sdf;
mod sticky;
mod subtree_builtins;
mod subtree_effect;
mod text_blur;
mod texture;
mod timed_text;
mod transition;

pub use backdrop::*;
pub use bloom::{
    BloomOptions, bloom_blur_shader, bloom_composite_shader, bloom_extract_shader, subtree_bloom,
};
pub use builtins::*;
pub use color_flow::{
    ColorFlow, ColorFlowOptions, ColorFlowPalette, ColorFlowPaletteColor, color_flow,
    color_flow_shader,
};
pub use contour_glow::{ContourGlowOptions, contour_glow_shader, subtree_contour_glow};
pub use contour_relief::{
    ContourReliefOptions, contour_relief_shader, contour_surface_wgsl, subtree_contour_relief,
};
pub use contour_shadow::{ContourShadowOptions, contour_shadow_shader, subtree_contour_shadow};
pub use deformation::{DeformationOptions, ElasticOffset, deformation_shader, subtree_deformation};
pub use depth_parallax::{DepthParallaxOptions, depth_parallax, depth_parallax_shader};
pub use displacement_map::{
    DisplacementMapOptions, DisplacementMapPreset, DisplacementMapSampling, DisplacementSourceEdge,
    displacement_map_shader, masked_displacement_map_shader, subtree_displacement_map,
};
pub use effect_stage::EffectStage;
pub use element::{Effect, effect, four_image_effect, image_effect, two_image_effect};
pub use feedback::{Feedback, FeedbackOptions, feedback_shader, subtree_feedback};
pub use flip::{
    FLIP_APPEARANCE_SLOT, FLIP_BACKGROUND_SLOT, FLIP_INTERACTION_SLOT, FLIP_LAYOUT_SLOT,
    FLIP_REGIONS_SLOT, Flip, FlipDirection, FlipEntry, FlipEvent, FlipImageRegion, FlipJumpResult,
    FlipLayout, FlipObjectFit, FlipPositionReason, FlipPreloadReason, FlipReadingDirection,
    FlipRequestResult, FlipSlot, FlipStyle, FlipUpdateResult, flip_shader, flip_shader_for,
    rigid_flip_shader, soft_flip_shader,
};
pub use fluid::{Fluid, FluidOptions, FluidSplat, fluid};
pub use glass::{FrostedGlass, FrostedGlassAppearance, FrostedGlassShape};
pub use lens::{LensOptions, lens_shader, subtree_lens};
pub use liquid_glass::{
    LiquidGlass, LiquidGlassAppearance, liquid_glass_shader, paint_liquid_glass,
};
pub use masked_builtins::{spectrum_mask_shader, spectrum_svg, spectrum_text};
pub use masked_effect::{MaskedEffect, effect_svg, effect_text, masked_effect};
pub use masked_fill::{MaskedFill, gradient_svg, gradient_text, masked_fill};
pub use material::{
    HolographicOptions, MaterialLight, MaterialSurface, holographic, holographic_image_shader,
    holographic_mask_shader, holographic_masked, holographic_shader,
};
pub use motion::{
    MotionEasing, MotionEvent, MotionFrame, MotionId, MotionItem, MotionLayer, MotionOptions,
    MotionPath, MotionPolicy,
};
pub use motion_blur::{MotionBlurOptions, motion_blur_shader, subtree_motion_blur};
pub use particle_transition::{ParticleTransitionOptions, subtree_particle_transition};
pub use particles::{
    ParticleMask, ParticlePhysics, ParticleSpawn, Particles, particles, subtree_particles,
};
pub use progressive_blur::{ProgressiveBlur, progressive_blur, progressive_blur_shader};
pub use ripple::{MAX_RIPPLES, Ripple, RippleOptions, ripple_shader, subtree_ripples};
pub use sdf::{MAX_SDF_SHAPES, SdfOptions, SdfScene, SdfShape, SdfTransform, sdf};
pub use sticky::{StickyShape, paint_sticky_shapes, sticky_shape_shader};
pub use subtree_builtins::{
    SubtreeColorOptions, SubtreeWaveOptions, subtree_blur, subtree_blur_shader,
    subtree_color_adjust, subtree_color_adjust_shader, subtree_identity, subtree_identity_shader,
    subtree_wave, subtree_wave_shader,
};
pub use subtree_effect::{SubtreeEffect, subtree_effect, subtree_effect_chain};
pub use text_blur::TextBlur;
pub use texture::{depth_fog_shader, hdr_tone_map_shader};
pub use timed_text::{TimedText, TimedTextEmphasis, TimedTextRevealWave, TimedTextUnit};
pub use transition::{SubtreeTransition, TransitionKind, subtree_transition, transition_shader};
