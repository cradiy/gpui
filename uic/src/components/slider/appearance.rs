use gpui::{Background, Hsla, Pixels, StyleRefinement, Styled, px, rgb};

/// Semantic colors and internal geometry for a slider.
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct SliderAppearance {
    pub active_track: Background,
    pub secondary_track: Background,
    pub thumb: Background,
    pub thumb_border: Hsla,
    pub focus_ring: Hsla,
    pub thumb_size: Pixels,
    #[chain(skip)]
    style: StyleRefinement,
}

/// Styled targets the visual track surface.
impl Styled for SliderAppearance {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Default for SliderAppearance {
    fn default() -> Self {
        Self {
            active_track: rgb(0x1677ff).into(),
            secondary_track: rgb(0x1677ff).opacity(0.25).into(),
            thumb: rgb(0xffffff).into(),
            thumb_border: rgb(0x1677ff).into(),
            focus_ring: rgb(0x1677ff).opacity(0.55).into(),
            thumb_size: px(18.),
            style: StyleRefinement::default()
                .w_full()
                .h(px(6.))
                .rounded_full()
                .bg(rgb(0x000000).opacity(0.14)),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{px, rgba};

    use super::*;

    #[test]
    fn track_surface_uses_styled_properties() {
        let appearance = SliderAppearance::default()
            .h(px(10.))
            .rounded(px(5.))
            .border_1()
            .bg(rgba(0x101820ff));

        assert!(appearance.style.size.height.is_some());
        assert!(appearance.style.background.is_some());
        assert!(appearance.style.border_widths.top.is_some());
        assert!(appearance.style.corner_radii.top_left.is_some());
    }
}
