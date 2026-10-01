use crate::{
    AnyElement, App, Bounds, Element, GlobalElementId, InspectorElementId, IntoElement, LayoutId,
    Pixels, Position, Style, Window,
};
use std::rc::Rc;

pub(crate) type OverlayRenderer = Rc<dyn Fn(&mut Window, &mut App) -> Option<AnyElement>>;

/// Builds a `Deferred` element, which delays the layout and paint of its child.
pub fn deferred(child: impl IntoElement) -> Deferred {
    Deferred {
        child: Some(child.into_any_element()),
        overlay: None,
        priority: 0,
    }
}

/// Builds a window-space overlay after ordinary elements have prepainted.
///
/// The renderer runs on every drawn frame, including when its containing view is
/// cached. It can query [`crate::ElementBounds`] to position the overlay using
/// current geometry. Return `None` when the anchor is unavailable. The overlay
/// does not contribute to its parent's layout and retains its parent's focus and
/// event ancestry. Its contents receive input in window coordinates.
pub fn deferred_overlay(
    render: impl Fn(&mut Window, &mut App) -> Option<AnyElement> + 'static,
) -> Deferred {
    Deferred {
        child: None,
        overlay: Some(Rc::new(render)),
        priority: 0,
    }
}

/// An element that schedules content after its ancestors.
/// [`deferred`] includes the child in the parent layout; [`deferred_overlay`]
/// constructs an independent window-space root during prepaint.
pub struct Deferred {
    child: Option<AnyElement>,
    overlay: Option<OverlayRenderer>,
    priority: usize,
}

impl Deferred {
    /// Sets the `priority` value of the `deferred` element, which
    /// determines the drawing order relative to other deferred elements,
    /// with higher values being drawn on top.
    pub fn with_priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}

impl Element for Deferred {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<crate::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let layout_id = if let Some(child) = self.child.as_mut() {
            child.request_layout(window, cx)
        } else {
            window.request_layout(
                Style {
                    position: Position::Absolute,
                    ..Default::default()
                },
                [],
                cx,
            )
        };
        (layout_id, ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) {
        let element_offset = window.element_offset();
        if let Some(render) = self.overlay.take() {
            window.defer_overlay(render, self.priority);
        } else {
            let child = self.child.take().unwrap();
            window.defer_draw(child, element_offset, self.priority, None);
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }
}

impl IntoElement for Deferred {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Deferred {
    /// Sets a priority for the element. A higher priority conceptually means painting the element
    /// on top of deferred draws with a lower priority (i.e. closer to the viewer).
    pub fn priority(mut self, priority: usize) -> Self {
        self.priority = priority;
        self
    }
}
