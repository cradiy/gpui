use std::{ops::RangeInclusive, time::Duration};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Background, ElementId, IntoElement,
    Refineable as _, RenderOnce, Role, SharedString, StyleRefinement, Styled, Window, div,
    prelude::*, px, relative, rgb,
};

use super::range::NumericRange;

/// Semantic colors for the internal progress layers.
#[derive(Clone, Copy, Debug, uic_macros::Chainable)]
pub struct ProgressAppearance {
    pub fill: Background,
    pub secondary_fill: Background,
    pub indeterminate_fill: Background,
}

impl Default for ProgressAppearance {
    fn default() -> Self {
        Self {
            fill: rgb(0x1677ff).into(),
            secondary_fill: rgb(0x1677ff).opacity(0.24).into(),
            indeterminate_fill: rgb(0x1677ff).into(),
        }
    }
}

/// A non-interactive determinate or indeterminate progress indicator.
///
/// The Styled surface is the outer track. Custom content is clipped to its
/// corresponding track, secondary, or primary layer.
#[derive(IntoElement)]
pub struct Progress {
    id: ElementId,
    label: Option<SharedString>,
    value: f64,
    range: NumericRange,
    secondary_value: Option<f64>,
    indeterminate: bool,
    appearance: ProgressAppearance,
    track_content: Option<AnyElement>,
    secondary_content: Option<AnyElement>,
    fill_content: Option<AnyElement>,
    style: StyleRefinement,
}

impl Progress {
    /// Creates a determinate progress indicator in the default `0.0..=1.0` range.
    pub fn new(id: impl Into<ElementId>, value: f64) -> Self {
        Self {
            id: id.into(),
            label: None,
            value,
            range: NumericRange::new(0.0..=1.0),
            secondary_value: None,
            indeterminate: false,
            appearance: ProgressAppearance::default(),
            track_content: None,
            secondary_content: None,
            fill_content: None,
            style: StyleRefinement::default()
                .w_full()
                .h(px(8.))
                .rounded_full()
                .bg(rgb(0x000000).opacity(0.12)),
        }
    }

    /// Creates an animated progress indicator without a numeric value.
    pub fn indeterminate(id: impl Into<ElementId>) -> Self {
        Self {
            indeterminate: true,
            ..Self::new(id, 0.0)
        }
    }

    /// Sets the numeric range. Both bounds must be finite and ordered.
    pub fn range(mut self, range: RangeInclusive<f64>) -> Self {
        self.range = NumericRange::new(range);
        self
    }

    /// Sets the primary value.
    pub fn value(mut self, value: f64) -> Self {
        self.value = value;
        self.indeterminate = false;
        self
    }

    /// Adds a secondary layer, such as buffered media or prefetched work.
    pub fn secondary_value(mut self, value: f64) -> Self {
        self.secondary_value = Some(value);
        self
    }

    pub fn appearance(mut self, appearance: ProgressAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Sets the accessible name of the progress indicator.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Adds arbitrary content over the complete track.
    pub fn track_content(mut self, content: impl IntoElement) -> Self {
        self.track_content = Some(content.into_any_element());
        self
    }

    /// Adds arbitrary content clipped to the secondary progress layer.
    pub fn secondary_content(mut self, content: impl IntoElement) -> Self {
        self.secondary_content = Some(content.into_any_element());
        self
    }

    /// Adds arbitrary content clipped to the primary progress layer.
    pub fn fill_content(mut self, content: impl IntoElement) -> Self {
        self.fill_content = Some(content.into_any_element());
        self
    }
}

impl RenderOnce for Progress {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let min = self.range.min();
        let max = self.range.max();
        let secondary = self.secondary_value.map(|value| {
            progress_layer(
                self.range.ratio(value),
                self.appearance.secondary_fill,
                self.secondary_content,
                &self.style,
            )
        });
        let primary: AnyElement = if self.indeterminate {
            let content = self.fill_content;
            let mut indicator = div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(relative(0.36))
                .overflow_hidden()
                .bg(self.appearance.indeterminate_fill)
                .children(content);
            indicator
                .style()
                .corner_radii
                .refine(&self.style.corner_radii);
            indicator
                .with_animation(
                    "uic-progress-indeterminate",
                    Animation::new(Duration::from_millis(1300)).repeat(),
                    |indicator, phase| {
                        let (visible_left, visible_width) = indeterminate_geometry(phase);
                        indicator
                            .left(relative(visible_left))
                            .w(relative(visible_width))
                    },
                )
                .into_any_element()
        } else {
            progress_layer(
                self.range.ratio(self.value),
                self.appearance.fill,
                self.fill_content,
                &self.style,
            )
            .into_any_element()
        };

        let mut element = div()
            .id(self.id)
            .relative()
            .role(Role::ProgressIndicator)
            .when_some(self.label, |this, label| this.aria_label(label))
            .aria_min_numeric_value(min)
            .aria_max_numeric_value(max)
            .when(!self.indeterminate, |this| {
                this.aria_numeric_value(self.range.clamp(self.value))
            })
            .children(self.track_content)
            .children(secondary)
            .child(primary);
        element.style().refine(&self.style);
        element.overflow_hidden()
    }
}

impl Styled for Progress {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn progress_layer(
    ratio: f32,
    background: Background,
    content: Option<AnyElement>,
    track_style: &StyleRefinement,
) -> gpui::Div {
    let mut layer = div()
        .absolute()
        .left_0()
        .top_0()
        .bottom_0()
        .w(relative(ratio))
        .overflow_hidden()
        .bg(background)
        .children(content);
    layer.style().corner_radii.refine(&track_style.corner_radii);
    layer
}

fn indeterminate_geometry(phase: f32) -> (f32, f32) {
    const SEGMENT: f32 = 0.36;
    let nominal_left = -SEGMENT + phase.clamp(0.0, 1.0) * (1.0 + SEGMENT);
    let visible_left = nominal_left.max(0.0);
    let visible_right = (nominal_left + SEGMENT).min(1.0);
    (visible_left, (visible_right - visible_left).max(0.0))
}

#[cfg(test)]
mod tests {
    use gpui::relative;

    use super::*;

    #[test]
    fn exposes_styled_and_custom_layer_content() {
        let _ = Progress::new("download-progress", 40.0)
            .range(0.0..=100.0)
            .secondary_value(70.0)
            .w(relative(0.8))
            .h(px(12.))
            .rounded(px(6.))
            .track_content(div().size_full())
            .secondary_content(div().size_full())
            .fill_content(div().size_full())
            .into_any_element();
    }

    #[test]
    fn indeterminate_segment_moves_forward_and_stays_inside_the_track() {
        let start = indeterminate_geometry(0.0);
        let middle = indeterminate_geometry(0.5);
        let end = indeterminate_geometry(1.0);

        assert_eq!(start, (0.0, 0.0));
        assert!(middle.0 > start.0);
        assert!(middle.1 > 0.0);
        assert_eq!(end, (1.0, 0.0));
    }
}
