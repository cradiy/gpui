/// A unique identifier for an element that can be inspected.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub struct InspectorElementId {
    /// Stable part of the ID.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub path: std::rc::Rc<InspectorElementPath>,
    /// Disambiguates elements that have the same path.
    #[cfg(any(feature = "inspector", debug_assertions))]
    pub instance_id: usize,
}

impl Into<InspectorElementId> for &InspectorElementId {
    fn into(self) -> InspectorElementId {
        self.clone()
    }
}

#[cfg(any(feature = "inspector", debug_assertions))]
pub use conditional::*;

#[cfg(any(feature = "inspector", debug_assertions))]
mod conditional {
    use super::*;
    use crate::{AnyElement, App, Context, Empty, IntoElement, Render, Window};
    use collections::{FxHashMap, TypeIdHashMap};
    use std::any::{Any, TypeId};

    /// A laid-out element in the inspected window. Parent indices refer to the same snapshot.
    #[derive(Clone, Debug)]
    pub struct InspectorElement {
        /// Identity shared with picking and registered state renderers.
        pub id: InspectorElementId,
        /// Nearest inspectable ancestor, or `None` for a root.
        pub parent: Option<usize>,
        /// Rust element type.
        pub type_name: &'static str,
        /// Layout bounds in logical window pixels, before subtree effects.
        pub bounds: crate::Bounds<crate::Pixels>,
        /// Inherited clipping rectangle in logical window pixels.
        pub content_mask: crate::ContentMask<crate::Pixels>,
        /// Computed style for elements backed by `Interactivity`.
        pub style: Option<crate::Style>,
        /// Resolved layout spacing, in logical pixels.
        pub box_model: InspectorBoxModel,
        /// Effective inherited text style, including this element's text refinements.
        pub text_style: crate::TextStyle,
        /// Logical pixel size of one rem at this element.
        pub rem_size: crate::Pixels,
        /// Text drawn directly within this element, excluding inspectable descendants.
        pub text: Vec<InspectorText>,
        /// Interaction configuration for elements backed by `Interactivity`.
        pub interaction: Option<InspectorInteraction>,
    }

    /// Interaction metadata captured before event listeners are consumed during paint.
    #[derive(Clone, Debug, Default)]
    pub struct InspectorInteraction {
        /// Whether this element needs a hitbox outside inspector picking mode.
        pub has_hitbox: bool,
        /// Whether the element tracks a focus handle.
        pub focusable: bool,
        /// Whether that handle currently owns keyboard focus.
        pub focused: bool,
        /// Whether that handle or one of its descendants owns keyboard focus.
        pub contains_focus: bool,
        /// Configured key context, when present.
        pub key_context: Option<String>,
        /// Explicit cursor request; `None` inherits normal cursor resolution.
        pub cursor: Option<crate::CursorStyle>,
        /// Registered listener counts, grouped by event type.
        pub listeners: Vec<(&'static str, usize)>,
        /// Current local scroll offset in logical pixels.
        pub scroll_offset: crate::Point<crate::Pixels>,
    }

    /// One hitbox containing a probe point, ordered front to back by the window API.
    #[derive(Clone, Debug)]
    pub struct InspectorHitbox {
        /// Owning or nearest inspectable element, if known.
        pub element: Option<InspectorElementId>,
        /// The hitbox including clipping, behavior and pointer coordinate mapping.
        pub hitbox: crate::Hitbox,
        /// Geometrically eligible for mouse input after occlusion, before event propagation.
        pub mouse: bool,
        /// Geometrically eligible for scrolling after occlusion, before event propagation.
        pub scroll: bool,
        /// Whether this hitbox currently holds pointer capture.
        pub captured: bool,
    }

    /// Spacing resolved by the layout engine, including percentages and auto margins.
    #[derive(Clone, Debug, Default)]
    pub struct InspectorBoxModel {
        /// Resolved outer spacing.
        pub margin: crate::Edges<crate::Pixels>,
        /// Resolved border widths.
        pub border: crate::Edges<crate::Pixels>,
        /// Resolved inner spacing.
        pub padding: crate::Edges<crate::Pixels>,
        /// Space reserved for scrollbars.
        pub scrollbar: crate::Size<crate::Pixels>,
    }

    /// A bounded preview of a text layout associated with an inspected element.
    #[derive(Clone, Debug)]
    pub struct InspectorText {
        /// First 256 Unicode scalar values of the source text.
        pub preview: crate::SharedString,
        /// Whether the source extends beyond the preview.
        pub preview_shortened: bool,
        /// Layout bounds in logical window coordinates.
        pub bounds: crate::Bounds<crate::Pixels>,
        /// Resolved base text style. Styled spans may override individual runs.
        pub base_style: crate::TextStyle,
        /// Computed line height in logical pixels.
        pub line_height: crate::Pixels,
    }

    /// `GlobalElementId` qualified by source location of element construction.
    #[derive(Debug, Eq, PartialEq, Hash)]
    pub struct InspectorElementPath {
        /// The path to the nearest ancestor element that has an `ElementId`.
        #[cfg(any(feature = "inspector", debug_assertions))]
        pub global_id: crate::GlobalElementId,
        /// Source location where this element was constructed.
        #[cfg(any(feature = "inspector", debug_assertions))]
        pub source_location: &'static std::panic::Location<'static>,
    }

    impl Clone for InspectorElementPath {
        fn clone(&self) -> Self {
            Self {
                global_id: self.global_id.clone(),
                source_location: self.source_location,
            }
        }
    }

    impl Into<InspectorElementPath> for &InspectorElementPath {
        fn into(self) -> InspectorElementPath {
            self.clone()
        }
    }

    /// Function set on `App` to render the inspector UI.
    pub type InspectorRenderer =
        Box<dyn Fn(&mut Inspector, &mut Window, &mut Context<Inspector>) -> AnyElement>;

    /// Manages inspector state - which element is currently selected and whether the inspector is
    /// in picking mode.
    pub struct Inspector {
        active_element: Option<InspectedElement>,
        pub(crate) pick_depth: Option<f32>,
        highlight: bool,
        pub(crate) pointer_position: Option<crate::Point<crate::Pixels>>,
    }

    struct InspectedElement {
        id: InspectorElementId,
        states: TypeIdHashMap<Box<dyn Any>>,
    }

    impl InspectedElement {
        fn new(id: InspectorElementId) -> Self {
            InspectedElement {
                id,
                states: Default::default(),
            }
        }
    }

    impl Inspector {
        pub(crate) fn new() -> Self {
            Self {
                active_element: None,
                pick_depth: Some(0.0),
                highlight: true,
                pointer_position: None,
            }
        }

        /// Selects an element and leaves picking mode.
        pub fn select(&mut self, id: InspectorElementId, window: &mut Window) {
            self.set_active_element_id(id, window);
            self.pick_depth = None;
            window.refresh();
        }

        pub(crate) fn hover(&mut self, id: InspectorElementId, window: &mut Window) {
            if self.is_picking() {
                let changed = self.set_active_element_id(id, window);
                if changed {
                    self.pick_depth = Some(0.0);
                }
            }
        }

        pub(crate) fn set_active_element_id(
            &mut self,
            id: InspectorElementId,
            window: &mut Window,
        ) -> bool {
            let changed = Some(&id) != self.active_element_id();
            if changed {
                self.active_element = Some(InspectedElement::new(id));
                window.refresh();
            }
            changed
        }

        /// ID of the currently hovered or selected element.
        pub fn active_element_id(&self) -> Option<&InspectorElementId> {
            self.active_element.as_ref().map(|e| &e.id)
        }

        pub(crate) fn with_active_element_state<T: 'static, R>(
            &mut self,
            window: &mut Window,
            f: impl FnOnce(&mut Option<T>, &mut Window) -> R,
        ) -> R {
            let Some(active_element) = &mut self.active_element else {
                return f(&mut None, window);
            };

            let type_id = TypeId::of::<T>();
            let mut inspector_state = active_element
                .states
                .remove(&type_id)
                .map(|state| *state.downcast().unwrap());

            let result = f(&mut inspector_state, window);

            if let Some(inspector_state) = inspector_state {
                active_element
                    .states
                    .insert(type_id, Box::new(inspector_state));
            }

            result
        }

        /// Starts element picking mode, allowing the user to select elements by clicking.
        pub fn start_picking(&mut self) {
            self.pick_depth = Some(0.0);
            self.highlight = true;
        }

        /// Whether the selected element's overlay is visible.
        pub fn is_highlighting(&self) -> bool {
            self.highlight
        }

        /// Last pointer position in the application area, retained while using the panel.
        pub fn pointer_position(&self) -> Option<crate::Point<crate::Pixels>> {
            self.pointer_position
        }

        /// Shows or hides the selection overlay without changing the selection.
        pub fn set_highlighting(&mut self, highlight: bool, window: &mut Window) {
            self.highlight = highlight;
            window.refresh();
        }

        /// Leaves picking mode without changing the selected element.
        pub fn stop_picking(&mut self) {
            self.pick_depth = None;
        }

        /// Returns whether the inspector is currently in picking mode.
        pub fn is_picking(&self) -> bool {
            self.pick_depth.is_some()
        }

        /// Renders elements for all registered inspector states of the active inspector element.
        pub fn render_inspector_states(
            &mut self,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) -> Vec<AnyElement> {
            let mut elements = Vec::new();
            if let Some(active_element) = self.active_element.take() {
                for (type_id, state) in &active_element.states {
                    if let Some(render_inspector) = cx
                        .inspector_element_registry
                        .renderers_by_type_id
                        .remove(type_id)
                    {
                        let mut element = (render_inspector)(
                            active_element.id.clone(),
                            state.as_ref(),
                            window,
                            cx,
                        );
                        elements.push(element);
                        cx.inspector_element_registry
                            .renderers_by_type_id
                            .insert(*type_id, render_inspector);
                    }
                }

                self.active_element = Some(active_element);
            }

            elements
        }
    }

    impl Render for Inspector {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            if let Some(inspector_renderer) = cx.inspector_renderer.take() {
                let result = inspector_renderer(self, window, cx);
                cx.inspector_renderer = Some(inspector_renderer);
                result
            } else {
                Empty.into_any_element()
            }
        }
    }

    #[derive(Default)]
    pub(crate) struct InspectorElementRegistry {
        renderers_by_type_id: FxHashMap<
            TypeId,
            Box<dyn Fn(InspectorElementId, &dyn Any, &mut Window, &mut App) -> AnyElement>,
        >,
    }

    impl InspectorElementRegistry {
        pub fn register<T: 'static, R: IntoElement>(
            &mut self,
            f: impl 'static + Fn(InspectorElementId, &T, &mut Window, &mut App) -> R,
        ) {
            self.renderers_by_type_id.insert(
                TypeId::of::<T>(),
                Box::new(move |id, value, window, cx| {
                    let value = value.downcast_ref().unwrap();
                    f(id, value, window, cx).into_any_element()
                }),
            );
        }
    }
}

/// Provides definitions used by `#[derive_inspector_reflection]`.
#[cfg(any(feature = "inspector", debug_assertions))]
pub mod inspector_reflection {
    use std::any::Any;

    /// Reification of a function that has the signature `fn some_fn(T) -> T`. Provides the name,
    /// documentation, and ability to invoke the function.
    #[derive(Clone, Copy)]
    pub struct FunctionReflection<T> {
        /// The name of the function
        pub name: &'static str,
        /// The method
        pub function: fn(Box<dyn Any>) -> Box<dyn Any>,
        /// Documentation for the function
        pub documentation: Option<&'static str>,
        /// `PhantomData` for the type of the argument and result
        pub _type: std::marker::PhantomData<T>,
    }

    impl<T: 'static> FunctionReflection<T> {
        /// Invoke this method on a value and return the result.
        pub fn invoke(&self, value: T) -> T {
            let boxed = Box::new(value) as Box<dyn Any>;
            let result = (self.function)(boxed);
            *result
                .downcast::<T>()
                .expect("Type mismatch in reflection invoke")
        }
    }
}
