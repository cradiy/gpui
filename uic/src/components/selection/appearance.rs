use gpui::{Background, Hsla, Pixels, px, rgb};

/// Semantic states and internal indicator geometry for a checkbox.
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct CheckboxAppearance {
    pub unchecked: Background,
    pub checked: Background,
    pub indeterminate: Background,
    pub indicator_border: Hsla,
    pub hover_border: Hsla,
    pub mark: Hsla,
    pub focus_ring: Hsla,
    pub indicator_size: Pixels,
    pub indicator_radius: Pixels,
    pub disabled_opacity: f32,
}

impl Default for CheckboxAppearance {
    fn default() -> Self {
        Self {
            unchecked: rgb(0xffffff).into(),
            checked: rgb(0x1677ff).into(),
            indeterminate: rgb(0x1677ff).into(),
            indicator_border: rgb(0x8c8c8c).into(),
            hover_border: rgb(0x1677ff).into(),
            mark: rgb(0xffffff).into(),
            focus_ring: rgb(0x1677ff).opacity(0.55).into(),
            indicator_size: px(18.),
            indicator_radius: px(4.),
            disabled_opacity: 0.5,
        }
    }
}

/// Semantic states and internal track/thumb geometry for a switch.
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct SwitchAppearance {
    pub off_track: Background,
    pub on_track: Background,
    pub hover_off_track: Background,
    pub hover_on_track: Background,
    pub thumb: Background,
    pub focus_ring: Hsla,
    pub thumb_size: Pixels,
    pub disabled_opacity: f32,
}

impl Default for SwitchAppearance {
    fn default() -> Self {
        Self {
            off_track: rgb(0xe2e5e9).into(),
            on_track: rgb(0x1677ff).into(),
            hover_off_track: rgb(0xd4d9e0).into(),
            hover_on_track: rgb(0x4096ff).into(),
            thumb: rgb(0xffffff).into(),
            focus_ring: rgb(0x1677ff).opacity(0.55).into(),
            thumb_size: px(16.),
            disabled_opacity: 0.5,
        }
    }
}

/// Semantic states and internal indicator geometry for a radio group.
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct RadioGroupAppearance {
    pub indicator: Background,
    pub indicator_border: Hsla,
    pub hover_border: Hsla,
    pub selected_border: Hsla,
    pub selected_dot: Background,
    pub focus_ring: Hsla,
    pub indicator_size: Pixels,
    pub dot_size: Pixels,
    pub disabled_opacity: f32,
}

impl Default for RadioGroupAppearance {
    fn default() -> Self {
        Self {
            indicator: rgb(0xffffff).into(),
            indicator_border: rgb(0x8c8c8c).into(),
            hover_border: rgb(0x1677ff).into(),
            selected_border: rgb(0x1677ff).into(),
            selected_dot: rgb(0x1677ff).into(),
            focus_ring: rgb(0x1677ff).opacity(0.55).into(),
            indicator_size: px(18.),
            dot_size: px(8.),
            disabled_opacity: 0.5,
        }
    }
}
