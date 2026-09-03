use gpui::{Hsla, rgb, rgba};

/// Semantic colors and states used inside [`super::Collapsible`].
///
/// The outer surface remains fully styleable through [`gpui::Styled`].
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct CollapsibleAppearance {
    pub indicator: Hsla,
    pub divider: Hsla,
    pub hover_background: Hsla,
    pub focus_ring: Hsla,
    pub disabled_opacity: f32,
}

impl Default for CollapsibleAppearance {
    fn default() -> Self {
        Self {
            indicator: rgb(0x667085).into(),
            divider: rgb(0xe7ebf0).into(),
            hover_background: rgba(0x4263eb08).into(),
            focus_ring: rgba(0x4263eb8f).into(),
            disabled_opacity: 0.48,
        }
    }
}
