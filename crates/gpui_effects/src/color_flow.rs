use gpui::{EffectShader, EffectUniforms, ImageSource, IntoElement, Rgba, StyleRefinement, Styled};

use crate::{Effect, image_effect};

const COLOR_SLOT: usize = 0;
const FLOW_SLOT: usize = 1;
const LIGHT_SLOT: usize = 2;
const PALETTE_START_SLOT: usize = 3;
const TONE_SLOT: usize = 7;

/// One image-derived color and its relative contribution to the background.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorFlowPaletteColor {
    /// RGB color. Alpha does not control palette weight.
    pub color: Rgba,
    /// Relative contribution in `0..=1`; zero excludes this color.
    pub weight: f32,
}

impl ColorFlowPaletteColor {
    pub fn new(color: impl Into<Rgba>, weight: f32) -> Self {
        Self {
            color: color.into(),
            weight,
        }
    }
}

/// Four main colors, each paired with its relative contribution.
pub type ColorFlowPalette = [ColorFlowPaletteColor; 4];

/// Visual parameters for [`ColorFlow`]. Layout and opacity use [`Styled`].
#[derive(Clone, Copy, Debug, PartialEq, uic_macros::Chainable)]
pub struct ColorFlowOptions {
    /// Color blending radius: `0..=0.3`, with larger values giving softer transitions.
    pub diffusion: f32,
    /// Color saturation multiplier. `0` is grayscale; `1` preserves source saturation.
    pub saturation: f32,
    /// Background brightness multiplier. `0` is black; `1` is normal brightness.
    pub brightness: f32,
    /// Image-derived dark base multiplier in `0..=1`; zero leaves a black base.
    pub shadow_level: f32,
    /// Soft RGB highlight ceiling in `0..=1`, before saturation and brightness.
    pub highlight_level: f32,
    /// Relative contribution of bright neutral colors in `0..=1`.
    pub neutral_weight: f32,
    /// Movement amplitude in `0..=2`. Zero freezes the color field.
    pub motion: f32,
    /// Amount of broad flow distortion in `0..=3`.
    pub flow_scale: f32,
    /// Travel amplitude in `0..=2`; at high cohesion this mainly moves the shared group.
    pub drift: f32,
    /// Shared movement and spatial confinement in `0..=1`; higher values keep colors together.
    pub cohesion: f32,
    /// Edge darkening strength in `0..=1`.
    pub vignette: f32,
    /// Stable variation of the arrangement and motion paths.
    pub seed: f32,
    /// Nonnegative light density; larger values reveal more of the local color over the base.
    pub glow: f32,
    /// Monochrome dithering in 8-bit color steps, `0..=2`; zero disables it.
    pub dither: f32,
}

impl Default for ColorFlowOptions {
    fn default() -> Self {
        Self {
            diffusion: 0.18,
            saturation: 1.0,
            brightness: 1.0,
            shadow_level: 0.85,
            highlight_level: 0.75,
            neutral_weight: 0.70,
            motion: 1.0,
            flow_scale: 1.0,
            drift: 0.45,
            cohesion: 0.85,
            vignette: 0.18,
            seed: 0.37,
            glow: 0.20,
            dither: 1.0,
        }
    }
}

impl ColorFlowOptions {
    fn uniforms(self, palette: Option<ColorFlowPalette>) -> EffectUniforms {
        let mut uniforms = EffectUniforms::new()
            .with_slot(
                COLOR_SLOT,
                [
                    self.diffusion,
                    self.saturation,
                    self.brightness,
                    self.motion,
                ],
            )
            .with_slot(
                FLOW_SLOT,
                [self.flow_scale, self.drift, self.vignette, self.seed],
            )
            .with_slot(
                LIGHT_SLOT,
                [
                    self.glow,
                    if palette.is_some() { 1.0 } else { 0.0 },
                    self.dither,
                    self.cohesion,
                ],
            )
            .with_slot(
                TONE_SLOT,
                [
                    self.shadow_level,
                    self.highlight_level,
                    self.neutral_weight,
                    0.0,
                ],
            );
        if let Some(palette) = palette {
            for (index, entry) in palette.into_iter().enumerate() {
                uniforms.set_slot(
                    PALETTE_START_SLOT + index,
                    [
                        entry.color.r.clamp(0.0, 1.0),
                        entry.color.g.clamp(0.0, 1.0),
                        entry.color.b.clamp(0.0, 1.0),
                        entry.weight.clamp(0.0, 1.0),
                    ],
                );
            }
        }
        uniforms
    }
}

/// Styled image-derived flowing light with named visual and palette configuration.
pub struct ColorFlow {
    effect: Effect,
    options: ColorFlowOptions,
    palette: Option<ColorFlowPalette>,
}

/// Creates a flowing-light background with default options and tone-grouped image sampling.
///
/// Use [`ColorFlow::options`] to configure its appearance and [`ColorFlow::palette`]
/// to supply extracted colors. Pass elapsed seconds to [`ColorFlow::time`].
pub fn color_flow(source: impl Into<ImageSource>) -> ColorFlow {
    ColorFlow::new(source)
}

impl ColorFlow {
    pub fn new(source: impl Into<ImageSource>) -> Self {
        Self {
            effect: image_effect(source, color_flow_shader()).bg(gpui::rgb(0x2a2834)),
            options: ColorFlowOptions::default(),
            palette: None,
        }
    }

    /// Sets the complete visual configuration without changing layout, time, or palette.
    pub fn options(mut self, options: ColorFlowOptions) -> Self {
        self.options = options;
        self
    }

    /// Supplies four weighted colors. `None` restores tone-grouped image sampling.
    pub fn palette(mut self, palette: impl Into<Option<ColorFlowPalette>>) -> Self {
        self.palette = palette.into();
        self
    }

    /// Sets elapsed animation seconds. Keep time fixed to pause the effect.
    pub fn time(mut self, seconds: f32) -> Self {
        self.effect = self.effect.time(seconds);
        self
    }

    /// Converts to the low-level effect for integration with generic effect APIs.
    pub fn into_effect(self) -> Effect {
        self.effect.uniforms(self.options.uniforms(self.palette))
    }
}

impl Styled for ColorFlow {
    fn style(&mut self) -> &mut StyleRefinement {
        self.effect.style()
    }
}

impl IntoElement for ColorFlow {
    type Element = Effect;

    fn into_element(self) -> Self::Element {
        self.into_effect()
    }
}

/// Returns the portable image-sampling shader used by [`ColorFlow`].
pub fn color_flow_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/color_flow.wgsl"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_options_map_to_shader_fields_without_palette_collisions() {
        let options = ColorFlowOptions::default()
            .diffusion(0.12)
            .saturation(0.8)
            .brightness(0.6)
            .shadow_level(0.4)
            .highlight_level(0.7)
            .neutral_weight(0.2)
            .motion(0.9)
            .flow_scale(1.3)
            .drift(0.7)
            .vignette(0.25)
            .seed(0.4)
            .glow(0.3)
            .dither(0.5)
            .cohesion(0.8);
        let palette = [ColorFlowPaletteColor::new(gpui::rgb(0xff0000), 0.25); 4];
        let uniforms = options.uniforms(Some(palette));
        assert_eq!(uniforms.slots()[0], [0.12, 0.8, 0.6, 0.9]);
        assert_eq!(uniforms.slots()[1], [1.3, 0.7, 0.25, 0.4]);
        assert_eq!(uniforms.slots()[2], [0.3, 1.0, 0.5, 0.8]);
        assert_eq!(uniforms.slots()[7], [0.4, 0.7, 0.2, 0.0]);
        for slot in &uniforms.slots()[3..7] {
            assert_eq!(*slot, [1.0, 0.0, 0.0, 0.25]);
        }
        let image_sampled = options.uniforms(None);
        assert_eq!(image_sampled.slots()[2], [0.3, 0.0, 0.5, 0.8]);
        assert_eq!(image_sampled.slots()[3..7], [[0.; 4]; 4]);
        assert_eq!(image_sampled.slots()[7], uniforms.slots()[7]);
    }
}
