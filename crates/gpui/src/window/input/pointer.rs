use crate::window::{
    CursorStyleRequest, DispatchPhase, Hitbox, HitboxBehavior, HitboxId, Window, WindowControlArea,
};
use crate::{
    App, Bounds, CursorStyle, DragEnd, MouseEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, px,
};
use std::any::Any;
use std::mem;

impl Window {
    /// Register a mouse event listener on the window for the next frame. The type of event
    /// is determined by the first parameter of the given listener. When the next frame is rendered
    /// the listener will be cleared.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn on_mouse_event<Event: MouseEvent>(
        &mut self,
        mut listener: impl FnMut(&Event, DispatchPhase, &mut Window, &mut App) + 'static,
    ) {
        self.invalidator.debug_assert_paint();
        let mapping = self.pointer_mapping.clone();
        self.next_frame.mouse_listeners.push(Some(Box::new(
            move |event: &dyn Any, phase: DispatchPhase, window: &mut Window, cx: &mut App| {
                if let Some(event) = event.downcast_ref::<Event>() {
                    if mapping.is_identity() {
                        let previous = mem::take(&mut window.pointer_mapping);
                        listener(event, phase, window, cx);
                        window.pointer_mapping = previous;
                        return;
                    }
                    let event = event.map_position(|position| mapping.map(position));
                    let previous = mem::replace(&mut window.pointer_mapping, mapping.clone());
                    listener(&event, phase, window, cx);
                    window.pointer_mapping = previous;
                }
            },
        )));
    }

    pub(in crate::window) fn dispatch_mouse_event(
        &mut self,
        event: &dyn Any,
        preserve_drag_on_mouse_up: bool,
        cx: &mut App,
    ) {
        let hit_test = self.rendered_frame.hit_test(self.raw_mouse_position());
        if hit_test != self.mouse_hit_test {
            self.mouse_hit_test = hit_test;
            self.reset_cursor_style(cx);
        }

        #[cfg(any(feature = "inspector", debug_assertions))]
        if self.is_inspector_picking(cx)
            && self.raw_mouse_position().x < self.viewport_size.width - self.inspector_width()
        {
            self.handle_inspector_mouse_event(event, cx);
            // When inspector is picking, all other mouse handling is skipped.
            return;
        }

        let mut mouse_listeners = mem::take(&mut self.rendered_frame.mouse_listeners);

        // Capture phase, events bubble from back to front. Handlers for this phase are used for
        // special purposes, such as detecting events outside of a given Bounds.
        for listener in &mut mouse_listeners {
            let listener = listener.as_mut().unwrap();
            listener(event, DispatchPhase::Capture, self, cx);
            if !cx.propagate_event {
                break;
            }
        }

        // Bubble phase, where most normal handlers do their work.
        if cx.propagate_event {
            for listener in mouse_listeners.iter_mut().rev() {
                let listener = listener.as_mut().unwrap();
                listener(event, DispatchPhase::Bubble, self, cx);
                if !cx.propagate_event {
                    break;
                }
            }
        }

        self.rendered_frame.mouse_listeners = mouse_listeners;

        if cx.has_active_drag() {
            if event.is::<MouseMoveEvent>() {
                // If this was a mouse move event, redraw the window so that the
                // active drag can follow the mouse cursor.
                self.refresh();
            } else if event.is::<MouseUpEvent>() && !preserve_drag_on_mouse_up {
                // If this was a mouse up event, cancel the active drag and redraw
                // the window.
                cx.finish_active_drag(DragEnd::Unaccepted, self);
            }
        }

        // Auto-release pointer capture on mouse up
        if event.is::<MouseUpEvent>() && self.captured_hitbox.is_some() {
            self.captured_hitbox = None;
        }
    }

    pub(in crate::window) fn reset_cursor_style(&self, cx: &mut App) {
        // Set the cursor only if we're the active window.
        if self.is_window_hovered() {
            let style = self
                .rendered_frame
                .cursor_style(self)
                .unwrap_or(CursorStyle::Arrow);
            cx.platform.set_cursor_style(style);
        }
    }

    /// The position of the mouse relative to the window, in the current pointer scope.
    /// Mapped mouse listeners receive source coordinates; use [`Self::raw_mouse_position`]
    /// for displayed window coordinates.
    pub fn mouse_position(&self) -> Point<Pixels> {
        self.pointer_mapping.map(self.mouse_position)
    }

    /// Pointer position in displayed window coordinates, outside any inverse mapping.
    pub fn raw_mouse_position(&self) -> Point<Pixels> {
        self.mouse_position
    }

    /// Scopes hitbox insertion and mouse listeners to a displayed-to-source mapping.
    /// Use the same transform around both prepaint and paint. Layout and drawing are unchanged.
    pub fn with_pointer_transform<R>(
        &mut self,
        bounds: Bounds<Pixels>,
        transform: crate::PointerTransform,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let bounds = self
            .snap_bounds(bounds)
            .map(|value| px(value.0 / self.scale_factor()));
        let mapping = self.pointer_mapping.then(
            bounds,
            self.content_mask().bounds,
            self.scale_factor(),
            transform,
        );
        let previous = mem::replace(&mut self.pointer_mapping, mapping);
        let result = f(self);
        self.pointer_mapping = previous;
        result
    }

    /// Captures the pointer for the given hitbox. While captured, all mouse move and mouse up
    /// events will be routed to listeners that check this hitbox's `is_hovered` status,
    /// regardless of actual hit testing. This enables drag operations that continue
    /// even when the pointer moves outside the element's bounds.
    ///
    /// The capture is automatically released on mouse up.
    pub fn capture_pointer(&mut self, hitbox_id: HitboxId) {
        self.captured_hitbox = Some(hitbox_id);
    }

    /// Releases any active pointer capture.
    pub fn release_pointer(&mut self) {
        self.captured_hitbox = None;
    }

    /// Returns the hitbox that has captured the pointer, if any.
    pub fn captured_hitbox(&self) -> Option<HitboxId> {
        self.captured_hitbox
    }

    /// Updates the cursor style at the platform level. This method should only be called
    /// during the paint phase of element drawing.
    pub fn set_cursor_style(&mut self, style: CursorStyle, hitbox: &Hitbox) {
        self.invalidator.debug_assert_paint();
        self.next_frame.cursor_styles.push(CursorStyleRequest {
            hitbox_id: Some(hitbox.id),
            style,
        });
    }

    /// Updates the cursor style for the entire window at the platform level. A cursor
    /// style using this method will have precedence over any cursor style set using
    /// `set_cursor_style`. This method should only be called during the paint
    /// phase of element drawing.
    pub fn set_window_cursor_style(&mut self, style: CursorStyle) {
        self.invalidator.debug_assert_paint();
        self.next_frame.cursor_styles.push(CursorStyleRequest {
            hitbox_id: None,
            style,
        })
    }

    /// This method should be called during `prepaint`. You can use
    /// the returned [Hitbox] during `paint` or in an event handler
    /// to determine whether the inserted hitbox was the topmost.
    ///
    /// This method should only be called as part of the prepaint phase of element drawing.
    pub fn insert_hitbox(&mut self, bounds: Bounds<Pixels>, behavior: HitboxBehavior) -> Hitbox {
        self.invalidator.debug_assert_prepaint();

        let content_mask = self.content_mask();
        let mut id = self.next_hitbox_id;
        self.next_hitbox_id = self.next_hitbox_id.next();
        let hitbox = Hitbox {
            pointer_mapping: self.pointer_mapping.clone(),
            id,
            bounds,
            content_mask,
            behavior,
        };
        self.next_frame.hitboxes.push(hitbox.clone());
        hitbox
    }

    /// Set a hitbox which will act as a control area of the platform window.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn insert_window_control_hitbox(&mut self, area: WindowControlArea, hitbox: Hitbox) {
        self.invalidator.debug_assert_paint();
        self.next_frame.window_control_hitboxes.push((area, hitbox));
    }
}
