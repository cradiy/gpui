use crate::{BackdropBlur, Bounds, EffectQuad, ScaledPixels};
use bytemuck::{Pod, Zeroable};

/// Rectangle layout used by effect shaders, in physical pixels.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ShaderBounds {
    /// Top-left coordinate.
    pub origin: [f32; 2],
    /// Width and height.
    pub size: [f32; 2],
}

impl From<Bounds<ScaledPixels>> for ShaderBounds {
    fn from(bounds: Bounds<ScaledPixels>) -> Self {
        Self {
            origin: [bounds.origin.x.0, bounds.origin.y.0],
            size: [bounds.size.width.0, bounds.size.height.0],
        }
    }
}

/// Affine transformation layout used by effect shaders.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ShaderTransformation {
    /// Rotation and scale data copied from [`crate::TransformationMatrix`].
    pub rotation_scale: [[f32; 2]; 2],
    /// Translation in physical pixels.
    pub translation: [f32; 2],
}

impl From<crate::TransformationMatrix> for ShaderTransformation {
    fn from(value: crate::TransformationMatrix) -> Self {
        Self {
            rotation_scale: value.rotation_scale,
            translation: value.translation,
        }
    }
}

/// GPU data for one effect, including four optional image regions.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct EffectInstance {
    /// Destination bounds before transformation.
    pub bounds: ShaderBounds,
    /// Bounds defining the effect's local coordinate system.
    pub effect_bounds: ShaderBounds,
    /// Transformation applied to the destination.
    pub transformation: ShaderTransformation,
    /// Window-space clipping rectangle.
    pub content_mask: ShaderBounds,
    /// Top-left, top-right, bottom-right and bottom-left radii.
    pub corner_radii: [f32; 4],
    /// First image region in texture pixels; zero if absent.
    pub image_bounds: ShaderBounds,
    /// Second image region in texture pixels; zero if absent.
    pub second_image_bounds: ShaderBounds,
    /// Third image region in texture pixels; zero if absent.
    pub third_image_bounds: ShaderBounds,
    /// Fourth image region in texture pixels; zero if absent.
    pub fourth_image_bounds: ShaderBounds,
    /// Opacity multiplier.
    pub opacity: f32,
    /// Animation time supplied to the shader.
    pub time: f32,
    /// Reserved, must be zero.
    pub pad: [f32; 2],
    /// Padding to align the uniform slots to 16 bytes; must be zero.
    pub alignment_pad: [f32; 2],
    /// User-supplied shader parameters.
    pub uniforms: [[f32; 4]; crate::EFFECT_UNIFORM_SLOTS],
}

impl From<&EffectQuad> for EffectInstance {
    fn from(effect: &EffectQuad) -> Self {
        Self {
            bounds: effect.bounds.into(),
            effect_bounds: effect.effect_bounds.into(),
            transformation: effect.transformation.into(),
            content_mask: effect.content_mask.bounds.into(),
            corner_radii: [
                effect.corner_radii.top_left.0,
                effect.corner_radii.top_right.0,
                effect.corner_radii.bottom_right.0,
                effect.corner_radii.bottom_left.0,
            ],
            image_bounds: effect
                .image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            second_image_bounds: effect
                .second_image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            third_image_bounds: effect
                .third_image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            fourth_image_bounds: effect
                .fourth_image_tile
                .map(|tile| tile.bounds.map(|value| ScaledPixels(value.0 as f32)).into())
                .unwrap_or_else(|| Bounds::<ScaledPixels>::default().into()),
            opacity: effect.opacity,
            time: effect.time,
            pad: [0.0; 2],
            alignment_pad: [0.0; 2],
            uniforms: *effect.uniforms.slots(),
        }
    }
}

/// GPU data for a backdrop effect or a separable blur pass.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct BackdropInstance {
    /// Destination rectangle.
    pub bounds: ShaderBounds,
    /// Clipping rectangle, or target dimensions for a blur pass.
    pub content_mask: ShaderBounds,
    /// Top-left, top-right, bottom-right and bottom-left radii.
    pub corner_radii: [f32; 4],
    /// Blur radius in physical pixels.
    pub blur_radius: f32,
    /// Opacity multiplier.
    pub opacity: f32,
    /// Animation time supplied to the shader.
    pub time: f32,
    /// One when the pointer is active, otherwise zero.
    pub pointer_active: f32,
    /// Blur sampling direction.
    pub direction: [f32; 2],
    /// Pointer position in normalized effect coordinates.
    pub pointer: [f32; 2],
    /// User-supplied shader parameters.
    pub uniforms: [[f32; 4]; crate::EFFECT_UNIFORM_SLOTS],
}

impl BackdropInstance {
    /// Creates a full-target blur pass; source and render dimensions may differ.
    pub fn blur(
        viewport_size: [f32; 2],
        render_size: [f32; 2],
        blur_radius: f32,
        direction: [f32; 2],
    ) -> Self {
        let viewport = ShaderBounds {
            origin: [0.0, 0.0],
            size: viewport_size,
        };
        Self {
            bounds: viewport,
            content_mask: ShaderBounds {
                origin: [0.0, 0.0],
                size: render_size,
            },
            corner_radii: [0.0; 4],
            blur_radius,
            opacity: 1.0,
            time: 0.0,
            pointer_active: 0.0,
            direction,
            pointer: [0.5; 2],
            uniforms: [[0.0; 4]; crate::EFFECT_UNIFORM_SLOTS],
        }
    }
}

impl From<&BackdropBlur> for BackdropInstance {
    fn from(backdrop: &BackdropBlur) -> Self {
        Self {
            bounds: backdrop.bounds.into(),
            content_mask: backdrop.content_mask.bounds.into(),
            corner_radii: [
                backdrop.corner_radii.top_left.0,
                backdrop.corner_radii.top_right.0,
                backdrop.corner_radii.bottom_right.0,
                backdrop.corner_radii.bottom_left.0,
            ],
            blur_radius: backdrop.blur_radius.0,
            opacity: backdrop.opacity,
            time: backdrop.time,
            pointer_active: u32::from(backdrop.pointer_active) as f32,
            direction: [0.0; 2],
            pointer: [backdrop.pointer.x, backdrop.pointer.y],
            uniforms: *backdrop.uniforms.slots(),
        }
    }
}
