use std::sync::{Arc, OnceLock};

use crate::{EffectStage, SubtreeEffect, subtree_effect_chain};
use gpui::{EffectShader, ImageSource, IntoElement, Pixels, Point, RenderImage, point, px};

/// Addressing for a displacement map outside its normalized coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplacementMapSampling {
    /// Extend edge texels.
    Clamp,
    /// Tile the map with bilinear filtering across tile seams.
    #[default]
    Repeat,
}

/// Source sampling beyond the captured content.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplacementSourceEdge {
    /// Pixels outside the capture are transparent.
    #[default]
    Transparent,
    /// Extend the capture's edge texels.
    Clamp,
}

/// RG displacement, texture coordinates and source boundary behavior.
#[derive(Clone, Copy, Debug)]
pub struct DisplacementMapOptions {
    /// Maximum source-sampling offset per axis, in logical pixels. Negative values reverse an axis.
    pub amplitude: Point<Pixels>,
    /// Map repetitions across the capture. Each axis is clamped to 0.01–64.
    pub scale: Point<f32>,
    /// Normalized map-coordinate offset.
    pub offset: Point<f32>,
    /// Map-coordinate movement per second, driven by the subtree's `time`.
    pub velocity: Point<f32>,
    /// Addressing of the displacement map.
    pub sampling: DisplacementMapSampling,
    /// Addressing of the captured source.
    pub source_edge: DisplacementSourceEdge,
}

impl Default for DisplacementMapOptions {
    fn default() -> Self {
        Self {
            amplitude: point(px(12.), px(12.)),
            scale: point(1., 1.),
            offset: point(0., 0.),
            velocity: point(0.035, -0.02),
            sampling: DisplacementMapSampling::Repeat,
            source_edge: DisplacementSourceEdge::Transparent,
        }
    }
}

/// Cached seamless RG displacement textures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplacementMapPreset {
    /// Broad overlapping waves.
    Water,
    /// Narrow vertical streams with gentle lateral distortion.
    Heat,
}

impl DisplacementMapPreset {
    /// Returns a shared, opaque control texture. R controls X and G controls Y.
    pub fn image(self) -> ImageSource {
        static WATER: OnceLock<Arc<RenderImage>> = OnceLock::new();
        static HEAT: OnceLock<Arc<RenderImage>> = OnceLock::new();
        let cache = match self {
            Self::Water => &WATER,
            Self::Heat => &HEAT,
        };
        cache
            .get_or_init(|| {
                let field = image::RgbaImage::from_fn(256, 256, |x, y| {
                    let u = (x as f32 + 0.5) / 256. * std::f32::consts::TAU;
                    let v = (y as f32 + 0.5) / 256. * std::f32::consts::TAU;
                    let (dx, dy) = match self {
                        Self::Water => (
                            (u + v).sin() * 0.45
                                + (2. * u - v + 0.7).sin() * 0.35
                                + (u + 3. * v).sin() * 0.2,
                            (u - v + 0.4).cos() * 0.45
                                + (u + 2. * v).sin() * 0.35
                                + (3. * u - v).cos() * 0.2,
                        ),
                        Self::Heat => (
                            (3. * u + 0.6 * v.sin()).sin() * 0.6
                                + (7. * u - v + 0.4).sin() * 0.25
                                + (11. * u + 2. * v).sin() * 0.15,
                            (2. * u - v).sin() * 0.65 + (5. * u + v).sin() * 0.35,
                        ),
                    };
                    let encode = |value: f32| (128. + 127. * value).round().clamp(0., 255.) as u8;
                    image::Rgba([0, encode(dy), encode(dx), 255])
                });
                Arc::new(RenderImage::new(smallvec::smallvec![image::Frame::new(
                    field
                )]))
            })
            .clone()
            .into()
    }

    /// Default motion and amplitude for this texture.
    pub fn options(self) -> DisplacementMapOptions {
        match self {
            Self::Water => DisplacementMapOptions::default(),
            Self::Heat => DisplacementMapOptions {
                amplitude: point(px(8.), px(3.)),
                velocity: point(0.005, 0.12),
                ..Default::default()
            },
        }
    }
}

/// Displaces a captured subtree using an external RG map.
pub fn subtree_displacement_map<E: IntoElement>(
    element: E,
    map: impl Into<ImageSource>,
    options: DisplacementMapOptions,
) -> SubtreeEffect<E::Element> {
    subtree_effect_chain(element, [EffectStage::displacement_map(map, options)])
}

impl EffectStage {
    /// Samples a map's RG channels for source offsets and alpha for local strength.
    /// Channel value 128 is neutral. The map is interpreted as data, without gamma conversion.
    pub fn displacement_map(map: impl Into<ImageSource>, options: DisplacementMapOptions) -> Self {
        displacement_stage(
            Self::with_images(displacement_map_shader(), [map.into()]),
            options,
        )
    }

    /// Multiplies displacement strength by a separate mask's red channel and alpha.
    /// The mask is stretched over the capture and does not move with the map.
    pub fn masked_displacement_map(
        map: impl Into<ImageSource>,
        mask: impl Into<ImageSource>,
        options: DisplacementMapOptions,
    ) -> Self {
        let map = map.into();
        let mask = mask.into();
        displacement_stage(
            Self::with_images(masked_displacement_map_shader(), [map.clone(), mask, map]),
            options,
        )
    }
}

fn displacement_stage(stage: EffectStage, options: DisplacementMapOptions) -> EffectStage {
    fn finite(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
        if value.is_finite() {
            value.clamp(min, max)
        } else {
            fallback
        }
    }
    let x = finite(options.amplitude.x.into(), 0., -256., 256.);
    let y = finite(options.amplitude.y.into(), 0., -256., 256.);
    stage
        .uniform_pixels(0, [px(x), px(y), px(0.), px(0.)])
        .uniform(
            1,
            [
                finite(options.scale.x, 1., 0.01, 64.),
                finite(options.scale.y, 1., 0.01, 64.),
                finite(options.offset.x, 0., -4096., 4096.),
                finite(options.offset.y, 0., -4096., 4096.),
            ],
        )
        .uniform(
            2,
            [
                finite(options.velocity.x, 0., -16., 16.),
                finite(options.velocity.y, 0., -16., 16.),
                f32::from(options.sampling == DisplacementMapSampling::Repeat),
                f32::from(options.source_edge == DisplacementSourceEdge::Clamp),
            ],
        )
        .capture_padding(px(x.abs().max(y.abs()) + 1.))
}

/// Two-image shader receiving captured content followed by an RG displacement map.
/// For subtree image stages. Slot 0: `[amplitude_x_px, amplitude_y_px, 0, 0]`;
/// slot 1: `[scale_x, scale_y, offset_x, offset_y]`;
/// slot 2: `[velocity_x, velocity_y, repeat_map, clamp_source]` (flags are 0 or 1).
pub fn displacement_map_shader() -> EffectShader {
    EffectShader::wgsl_two_images(concat!(
        "fn displacement_mask(input: EffectInput) -> f32 { return 1.0; }\n",
        include_str!("shaders/displacement_map.wgsl")
    ))
}

/// Four-image variant: source, map, local mask, and a reserved image input.
/// Uses the same uniform slots as [`displacement_map_shader`].
pub fn masked_displacement_map_shader() -> EffectShader {
    EffectShader::wgsl_four_images(concat!(
        "fn displacement_mask(input: EffectInput) -> f32 { let mask = sample_effect_third_image(input, input.uv); return mask.r * mask.a; }\n",
        include_str!("shaders/displacement_map.wgsl")
    ))
}
