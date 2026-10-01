use gpui::{
    AnyElement, App, Bounds, ElementBounds, IntoElement, Pixels, Point, Window, canvas, prelude::*,
};

#[derive(Clone, Default)]
pub(super) struct TriggerAnchor(ElementBounds);

impl TriggerAnchor {
    pub fn bounds(&self, window: &Window) -> Option<Bounds<Pixels>> {
        self.0.visible_bounds(window)?;
        self.0.bounds(window)
    }

    pub fn contains(&self, position: Point<Pixels>, window: &Window) -> bool {
        self.0.contains(position, window)
    }

    pub fn tracker(&self) -> impl IntoElement + use<> {
        let tracker = self.clone();
        canvas(
            move |bounds, window, _| {
                window.track_element_bounds(&tracker.0, bounds);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0()
    }
}

// Resolve after ordinary prepaint, so triggers and parent menu rows have their
// current-frame bounds. The returned surface lives entirely in window coordinates.
pub(super) fn resolve_overlay(
    resolve: impl FnOnce(&mut Window, &mut App) -> Option<AnyElement> + 'static,
) -> impl IntoElement {
    canvas(
        move |_, window, cx| {
            resolve(window, cx).map(|mut element| {
                element.prepaint_as_root(
                    Point::default(),
                    window.viewport_size().into(),
                    window,
                    cx,
                );
                element
            })
        },
        |_, element, window, cx| {
            if let Some(mut element) = element {
                element.paint(window, cx);
            }
        },
    )
    .absolute()
}

#[cfg(test)]
mod tests;
