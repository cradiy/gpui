use std::{cell::RefCell, rc::Rc};

use gpui::{
    AnyElement, App, Bounds, IntoElement, Pixels, Point, PointerMapping, Window, canvas, prelude::*,
};

#[derive(Clone, Default)]
pub(super) struct TriggerAnchor(Rc<RefCell<Option<(Bounds<Pixels>, PointerMapping)>>>);

impl TriggerAnchor {
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.0
            .borrow()
            .as_ref()
            .map(|(bounds, mapping)| mapping.bounds_to_display(*bounds).unwrap_or(*bounds))
    }

    pub fn contains(&self, position: Point<Pixels>) -> bool {
        self.0.borrow().as_ref().is_some_and(|(bounds, mapping)| {
            mapping
                .hit_position(position)
                .is_some_and(|point| bounds.contains(&point))
        })
    }

    pub fn same_trigger(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    pub fn tracker(&self) -> impl IntoElement + use<> {
        let tracker = self.clone();
        canvas(
            move |bounds, window, _| {
                *tracker.0.borrow_mut() = Some((bounds, window.pointer_mapping().clone()));
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
