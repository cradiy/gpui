use crate::prelude::*;
use crate::window::{FocusId, Window};
use crate::{
    ActiveDrag, AnyDrag, App, DragEnd, DragOrigin, DragPhase, FileDropEvent, InternalDragEvent,
    Keystroke, Modifiers, MouseButton, MouseMoveEvent, MouseUpEvent, PlatformInput, Task,
};
#[cfg(feature = "input-latency-histogram")]
use scheduler::Instant;
use smallvec::SmallVec;
use std::sync::Arc;

mod actions;
mod keyboard;
mod pointer;

/// Represents the two different phases when dispatching events.
#[derive(Default, Copy, Clone, Debug, Eq, PartialEq)]
pub enum DispatchPhase {
    /// After the capture phase comes the bubble phase, in which mouse event listeners are
    /// invoked front to back and keyboard event listeners are invoked from the focused element
    /// to the root of the element tree. This is the phase you'll most commonly want to use when
    /// registering event listeners.
    #[default]
    Bubble,
    /// During the initial capture phase, mouse event listeners are invoked back to front, and keyboard
    /// listeners are invoked from the root of the tree downward toward the focused element. This phase
    /// is used for special purposes such as clearing the "pressed" state for click events. If
    /// you stop event propagation during this phase, you need to know what you're doing. Handlers
    /// outside of the immediate region may rely on detecting non-local events during this phase.
    Capture,
}

impl DispatchPhase {
    /// Returns true if this represents the "bubble" phase.
    #[inline]
    pub fn bubble(self) -> bool {
        self == DispatchPhase::Bubble
    }

    /// Returns true if this represents the "capture" phase.
    #[inline]
    pub fn capture(self) -> bool {
        self == DispatchPhase::Capture
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(missing_docs)]
pub struct DispatchEventResult {
    pub propagate: bool,
    pub default_prevented: bool,
    /// Whether a process-local native drop ran a typed target handler.
    pub drag_drop_accepted: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd)]
pub(super) enum InputModality {
    Mouse,
    Keyboard,
    Touch,
}
#[derive(Clone, Debug, Default)]
pub(super) struct ModifierState {
    pub(super) modifiers: Modifiers,
    pub(super) saw_keystroke: bool,
}
#[derive(Default, Debug)]
pub(super) struct PendingInput {
    pub(super) keystrokes: SmallVec<[Keystroke; 1]>,
    pub(super) focus: Option<FocusId>,
    pub(super) timer: Option<Task<()>>,
    pub(super) needs_timeout: bool,
}

impl Window {
    /// Dispatch a mouse or keyboard event on the window.
    #[profiling::function]
    pub fn dispatch_event(&mut self, event: PlatformInput, cx: &mut App) -> DispatchEventResult {
        #[cfg(feature = "input-latency-histogram")]
        let dispatch_time = Instant::now();
        let update_count_before = self.invalidator.update_count();
        // Track input modality for focus-visible styling and hover suppression.
        // Hover is suppressed during keyboard modality so that keyboard navigation
        // doesn't show hover highlights on the item under the mouse cursor.
        let old_modality = self.last_input_modality;
        self.last_input_modality = match &event {
            PlatformInput::KeyDown(_) => InputModality::Keyboard,
            PlatformInput::MouseMove(_) | PlatformInput::MouseDown(_) => InputModality::Mouse,
            PlatformInput::Touch(_) => InputModality::Touch,
            _ => self.last_input_modality,
        };
        if self.last_input_modality != old_modality {
            self.refresh();
        }

        // Handlers may set this to false by calling `stop_propagation`.
        cx.propagate_event = true;
        // Handlers may set this to true by calling `prevent_default`.
        self.default_prevented = false;
        let mut refresh_native_drag_icon = None;

        // Once a drag is promoted, Wayland's data-device protocol owns release routing. The
        // pointer release still reaches the source wl_pointer, but dispatching it to normal GPUI
        // listeners would race the target drop and could run source-side fallback behavior.
        if let PlatformInput::MouseUp(mouse_up) = &event
            && cx.active_drag.as_ref().is_some_and(|drag| {
                matches!(
                    &drag.origin,
                    DragOrigin::Internal(session)
                        if matches!(
                            session.phase,
                            crate::DragPhase::Native | crate::DragPhase::Finishing
                        )
                            && session.source_window == self.handle.id
                )
            })
        {
            self.mouse_position = mouse_up.position;
            self.modifiers = mouse_up.modifiers;
            return DispatchEventResult {
                propagate: true,
                default_prevented: false,
                drag_drop_accepted: false,
            };
        }

        let mut preserve_drag_on_mouse_up = false;
        let mut dropped_session = None;

        let event = match event {
            // Track the mouse position with our own state, since accessing the platform
            // API for the mouse position can only occur on the main thread.
            PlatformInput::MouseMove(mouse_move) => {
                self.mouse_position = mouse_move.position;
                self.modifiers = mouse_move.modifiers;
                PlatformInput::MouseMove(mouse_move)
            }
            PlatformInput::MouseDown(mouse_down) => {
                self.mouse_position = mouse_down.position;
                self.modifiers = mouse_down.modifiers;
                PlatformInput::MouseDown(mouse_down)
            }
            PlatformInput::MouseUp(mouse_up) => {
                self.mouse_position = mouse_up.position;
                self.modifiers = mouse_up.modifiers;
                PlatformInput::MouseUp(mouse_up)
            }
            PlatformInput::MousePressure(mouse_pressure) => {
                PlatformInput::MousePressure(mouse_pressure)
            }
            PlatformInput::MouseExited(mouse_exited) => {
                self.modifiers = mouse_exited.modifiers;
                PlatformInput::MouseExited(mouse_exited)
            }
            PlatformInput::ModifiersChanged(modifiers_changed) => {
                self.modifiers = modifiers_changed.modifiers;
                self.capslock = modifiers_changed.capslock;
                PlatformInput::ModifiersChanged(modifiers_changed)
            }
            PlatformInput::ScrollWheel(scroll_wheel) => {
                self.mouse_position = scroll_wheel.position;
                self.modifiers = scroll_wheel.modifiers;
                PlatformInput::ScrollWheel(scroll_wheel)
            }
            PlatformInput::Pinch(pinch) => {
                self.mouse_position = pinch.position;
                self.modifiers = pinch.modifiers;
                PlatformInput::Pinch(pinch)
            }
            // Translate dragging and dropping of external files from the operating system
            // to internal drag and drop events.
            PlatformInput::FileDrop(file_drop) => match file_drop {
                FileDropEvent::Entered { position, paths } => {
                    self.mouse_position = position;
                    if cx.active_drag.is_none() {
                        cx.active_drag = Some(ActiveDrag {
                            data: AnyDrag {
                                value: Arc::new(paths.clone()),
                                view: cx.new(|_| paths).into(),
                                cursor_offset: position,
                                cursor_style: None,
                            },
                            origin: DragOrigin::ExternalFiles,
                        });
                    }
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
                FileDropEvent::Pending { position } => {
                    self.mouse_position = position;
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
                FileDropEvent::Submit { position } => {
                    cx.activate(true);
                    self.mouse_position = position;
                    PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    })
                }
                FileDropEvent::Exited => {
                    cx.active_drag.take();
                    PlatformInput::FileDrop(FileDropEvent::Exited)
                }
            },
            PlatformInput::InternalDrag(drag_event) => match drag_event {
                InternalDragEvent::Entered {
                    session_id,
                    position,
                }
                | InternalDragEvent::Moved {
                    session_id,
                    position,
                } => {
                    let matches_session = cx.active_drag.as_ref().is_some_and(|drag| {
                        matches!(
                            &drag.origin,
                            DragOrigin::Internal(session)
                                if session.session_id == session_id
                                    && session.phase == crate::DragPhase::Native
                        )
                    });
                    if !matches_session {
                        return DispatchEventResult::default();
                    }
                    refresh_native_drag_icon = Some(session_id);
                    self.mouse_position = position;
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
                InternalDragEvent::Left { session_id } => {
                    let matches_session = cx.active_drag.as_ref().is_some_and(|drag| {
                        matches!(
                            &drag.origin,
                            DragOrigin::Internal(session) if session.session_id == session_id
                        )
                    });
                    if !matches_session {
                        return DispatchEventResult::default();
                    }
                    refresh_native_drag_icon = Some(session_id);
                    PlatformInput::MouseExited(crate::MouseExitEvent {
                        position: self.mouse_position,
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
                InternalDragEvent::Dropped {
                    session_id,
                    position,
                } => {
                    let matches_session = cx.active_drag.as_ref().is_some_and(|drag| {
                        matches!(
                            &drag.origin,
                            DragOrigin::Internal(session)
                                if session.session_id == session_id
                                    && session.phase == crate::DragPhase::Native
                        )
                    });
                    if !matches_session {
                        return DispatchEventResult::default();
                    }
                    self.mouse_position = position;
                    preserve_drag_on_mouse_up = true;
                    dropped_session = Some(session_id);
                    PlatformInput::MouseUp(MouseUpEvent {
                        button: MouseButton::Left,
                        position,
                        modifiers: Modifiers::default(),
                        click_count: 1,
                    })
                }
                InternalDragEvent::SourceDropPerformed { session_id } => {
                    if let Some(active_drag) = cx.active_drag.as_mut()
                        && let DragOrigin::Internal(session) = &mut active_drag.origin
                        && session.session_id == session_id
                    {
                        session.drop_performed = true;
                    }
                    PlatformInput::InternalDrag(InternalDragEvent::SourceDropPerformed {
                        session_id,
                    })
                }
                InternalDragEvent::SourceFinished { session_id } => {
                    let outcome = cx.active_drag.as_ref().and_then(|drag| match &drag.origin {
                        DragOrigin::Internal(session) if session.session_id == session_id => {
                            Some(session.pending_outcome.unwrap_or(DragEnd::Unaccepted))
                        }
                        _ => None,
                    });
                    if let Some(outcome) = outcome {
                        cx.finish_active_drag(outcome, self);
                    }
                    PlatformInput::InternalDrag(InternalDragEvent::SourceFinished { session_id })
                }
                InternalDragEvent::SourceCancelled { session_id } => {
                    let outcome = cx.active_drag.as_ref().and_then(|drag| match &drag.origin {
                        DragOrigin::Internal(session) if session.session_id == session_id => {
                            Some(if session.drop_performed {
                                DragEnd::Unaccepted
                            } else {
                                DragEnd::Cancelled
                            })
                        }
                        _ => None,
                    });
                    if let Some(outcome) = outcome {
                        cx.finish_active_drag(outcome, self);
                    }
                    PlatformInput::InternalDrag(InternalDragEvent::SourceCancelled { session_id })
                }
            },
            PlatformInput::Touch(touch) => PlatformInput::Touch(touch),
            PlatformInput::KeyDown(_) | PlatformInput::KeyUp(_) => event,
        };

        if let Some(any_mouse_event) = event.mouse_event() {
            self.dispatch_mouse_event(any_mouse_event, preserve_drag_on_mouse_up, cx);
        } else if let Some(any_key_event) = event.keyboard_event() {
            self.dispatch_key_event(any_key_event, cx);
        }

        if let Some(session_id) = refresh_native_drag_icon {
            let source_window = cx.active_drag.as_ref().and_then(|drag| match &drag.origin {
                DragOrigin::Internal(session) if session.session_id == session_id => {
                    Some(session.source_window)
                }
                _ => None,
            });
            if source_window == Some(self.handle.id) {
                self.update_native_drag_icon_now(session_id, cx);
            } else if let Some(source_window) = source_window
                && let Some(handle) = cx.window_handles.get(&source_window).copied()
            {
                let _ = handle.update(cx, move |_, window, cx| {
                    window.update_native_drag_icon_now(session_id, cx);
                });
            }
        }

        if self.invalidator.update_count() > update_count_before {
            self.input_rate_tracker.borrow_mut().record_input();
            #[cfg(feature = "input-latency-histogram")]
            if self.invalidator.not_drawing() {
                self.input_latency_tracker.record_input(dispatch_time);
            } else {
                self.input_latency_tracker.record_mid_draw_input();
            }
        }

        DispatchEventResult {
            propagate: cx.propagate_event,
            default_prevented: self.default_prevented,
            drag_drop_accepted: dropped_session.is_some_and(|session_id| {
                cx.active_drag
                    .as_ref()
                    .is_none_or(|drag| match &drag.origin {
                        DragOrigin::Internal(session) if session.session_id == session_id => {
                            session.phase == DragPhase::Finishing
                                && matches!(session.pending_outcome, Some(DragEnd::Dropped { .. }))
                        }
                        _ => true,
                    })
            }),
        }
    }

    /// Call to prevent the default action of an event. Currently only used to prevent
    /// parent elements from becoming focused on mouse down.
    pub fn prevent_default(&mut self) {
        self.default_prevented = true;
    }

    /// Obtain whether default has been prevented for the event currently being dispatched.
    pub fn default_prevented(&self) -> bool {
        self.default_prevented
    }

    /// Returns true if the last input event was keyboard-based (key press, tab navigation, etc.)
    /// This is used for focus-visible styling to show focus indicators only for keyboard navigation.
    pub fn last_input_was_keyboard(&self) -> bool {
        self.last_input_modality == InputModality::Keyboard
    }
}
