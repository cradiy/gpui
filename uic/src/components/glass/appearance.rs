use gpui::{Hsla, hsla, px, rgb};
use gpui_effects::LiquidGlassAppearance;

/// Optical surfaces and semantic colors for a glass segmented control.
/// Layout, typography, outer corners, background, and shadow use `Styled`.
#[derive(Clone, Copy, Debug)]
pub struct GlassSegmentedAppearance {
    pub surface: LiquidGlassAppearance,
    pub selection: LiquidGlassAppearance,
    pub selected_text: Hsla,
    pub focus_ring: Hsla,
    pub disabled_opacity: f32,
}

impl GlassSegmentedAppearance {
    pub fn light() -> Self {
        Self {
            surface: LiquidGlassAppearance::regular()
                .blur_radius(px(5.))
                .clarity(0.5)
                .refraction(px(1.5))
                .thickness(px(10.))
                .highlight(0.2)
                .tint(hsla(0., 0., 1., 0.16)),
            selection: LiquidGlassAppearance::clear()
                .blur_radius(px(0.))
                .clarity(1.)
                .refraction(px(4.))
                .thickness(px(10.))
                .highlight(0.65)
                .dispersion(0.005)
                .tint(hsla(0., 0., 1., 0.28)),
            selected_text: rgb(0x172c40).into(),
            focus_ring: rgb(0x4a82bf).into(),
            disabled_opacity: 0.45,
        }
    }

    /// Pair with a light foreground color through `Styled::text_color`.
    pub fn dark() -> Self {
        Self {
            surface: LiquidGlassAppearance::dark()
                .blur_radius(px(5.))
                .clarity(0.5)
                .refraction(px(1.5))
                .thickness(px(10.))
                .highlight(0.18),
            selection: LiquidGlassAppearance::clear()
                .blur_radius(px(0.))
                .clarity(1.)
                .refraction(px(4.))
                .thickness(px(10.))
                .highlight(0.5)
                .dispersion(0.005)
                .tint(hsla(0.6, 0.15, 0.85, 0.16)),
            selected_text: rgb(0xffffff).into(),
            focus_ring: rgb(0x9bc8f5).into(),
            ..Self::light()
        }
    }
}

impl Default for GlassSegmentedAppearance {
    fn default() -> Self {
        Self::light()
    }
}
