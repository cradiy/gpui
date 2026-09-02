use gpui::{
    AnyElement, App, ElementId, Hsla, IntoElement, RenderOnce, Rgba, StyleRefinement, Styled,
    Window, checkerboard, div, hsla, prelude::*, px,
};

/// Preset geometry for a [`ColorPickerTrigger`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorPickerTriggerSize {
    Small,
    #[default]
    Medium,
    Large,
}

#[derive(Clone, Copy)]
struct TriggerMetrics {
    height: gpui::Pixels,
    swatch: gpui::Pixels,
    padding: gpui::Pixels,
    radius: gpui::Pixels,
    text_size: gpui::Pixels,
}

impl ColorPickerTriggerSize {
    fn metrics(self) -> TriggerMetrics {
        match self {
            Self::Small => TriggerMetrics {
                height: px(28.),
                swatch: px(18.),
                padding: px(4.),
                radius: px(6.),
                text_size: px(13.),
            },
            Self::Medium => TriggerMetrics {
                height: px(36.),
                swatch: px(24.),
                padding: px(5.),
                radius: px(8.),
                text_size: px(14.),
            },
            Self::Large => TriggerMetrics {
                height: px(44.),
                swatch: px(32.),
                padding: px(5.),
                radius: px(9.),
                text_size: px(16.),
            },
        }
    }
}

/// Semantic states and swatch optics for a [`ColorPickerTrigger`].
#[derive(Clone, Copy, Debug, uic_macros::Chainable)]
pub struct ColorPickerTriggerAppearance {
    pub border: Hsla,
    pub hover_border: Hsla,
    pub active_border: Hsla,
    pub swatch_border: Hsla,
    pub checker: Hsla,
    pub disabled_opacity: f32,
}

impl Default for ColorPickerTriggerAppearance {
    fn default() -> Self {
        Self {
            border: hsla(0.52, 0.08, 0.30, 1.0),
            hover_border: hsla(0.52, 0.08, 0.46, 1.0),
            active_border: hsla(0.59, 0.88, 0.56, 1.0),
            swatch_border: hsla(0.0, 0.0, 1.0, 0.22),
            checker: hsla(0.0, 0.0, 0.55, 0.42),
            disabled_opacity: 0.5,
        }
    }
}

/// A styled color swatch surface intended for use as a popover trigger.
///
/// This component owns presentation only. Wrap it in [`crate::components::popover::Popover`]
/// to provide button semantics, keyboard activation, and anchored picker content.
#[derive(IntoElement)]
pub struct ColorPickerTrigger {
    id: ElementId,
    value: Rgba,
    size: ColorPickerTriggerSize,
    label: Option<AnyElement>,
    show_value: bool,
    active: bool,
    disabled: bool,
    appearance: ColorPickerTriggerAppearance,
    style: StyleRefinement,
}

impl ColorPickerTrigger {
    pub fn new(id: impl Into<ElementId>, value: Rgba) -> Self {
        Self {
            id: id.into(),
            value,
            size: ColorPickerTriggerSize::default(),
            label: None,
            show_value: false,
            active: false,
            disabled: false,
            appearance: ColorPickerTriggerAppearance::default(),
            style: StyleRefinement::default()
                .bg(hsla(0.52, 0.10, 0.15, 1.0))
                .text_color(hsla(0.0, 0.0, 0.88, 1.0)),
        }
    }

    pub fn control_size(mut self, size: ColorPickerTriggerSize) -> Self {
        self.size = size;
        self
    }

    pub fn show_value(mut self, show: bool) -> Self {
        self.show_value = show;
        self
    }

    pub fn label(mut self, label: impl IntoElement) -> Self {
        self.label = Some(label.into_any_element());
        self
    }

    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn appearance(mut self, appearance: ColorPickerTriggerAppearance) -> Self {
        self.appearance = appearance;
        self
    }
}

impl RenderOnce for ColorPickerTrigger {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let metrics = self.size.metrics();
        let appearance = self.appearance;
        let label = self.label.or_else(|| {
            self.show_value
                .then(|| div().child(format_color(self.value)).into_any_element())
        });
        let swatch = div()
            .flex_none()
            .size(metrics.swatch)
            .rounded((metrics.radius - px(2.)).max(px(2.)))
            .border_1()
            .border_color(appearance.swatch_border)
            .overflow_hidden()
            .bg(checkerboard(appearance.checker, 4.0))
            .child(div().size_full().bg(self.value));

        let mut element = div()
            .id(self.id)
            .debug_selector(|| "color-picker-trigger".to_string())
            .h(metrics.height)
            .p(metrics.padding)
            .gap_2()
            .rounded(metrics.radius)
            .border(if self.active { px(2.) } else { px(1.) })
            .border_color(if self.active {
                appearance.active_border
            } else {
                appearance.border
            })
            .text_size(metrics.text_size)
            .opacity(if self.disabled {
                appearance.disabled_opacity
            } else {
                1.0
            })
            .flex()
            .items_center()
            .child(swatch)
            .children(label);
        element.style().refine(&self.style);
        if !self.disabled {
            element = element.hover(move |style| style.border_color(appearance.hover_border));
        }
        element
    }
}

impl Styled for ColorPickerTrigger {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn format_color(color: Rgba) -> String {
    let [r, g, b, a] = [color.r, color.g, color.b, color.a]
        .map(|channel| (channel.clamp(0.0, 1.0) * 255.0).round() as u8);
    if a == u8::MAX {
        format!("#{r:02X}{g:02X}{b:02X}")
    } else {
        format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
    }
}

#[cfg(test)]
mod tests {
    use gpui::rgba;

    use super::*;

    #[test]
    fn value_text_keeps_non_opaque_alpha() {
        assert_eq!(format_color(rgba(0x1677ff80)), "#1677FF80");
        assert_eq!(format_color(rgba(0x1677ffff)), "#1677FF");
    }
}
