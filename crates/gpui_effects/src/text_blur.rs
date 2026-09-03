use std::{cell::RefCell, rc::Rc};

use gpui::{
    App, Bounds, Element, ElementId, GlobalElementId, Hitbox, InspectorElementId,
    InteractiveElement, Interactivity, IntoElement, LayoutId, Pixels, Point, ShapedLine,
    SharedString, StyleRefinement, Styled, Window, point, px, size,
};

#[derive(Clone, Default)]
pub struct TextBlurLayout(Rc<RefCell<Option<TextBlurLayoutInner>>>);

struct TextBlurLayoutInner {
    line: ShapedLine,
    line_height: Pixels,
    content_bounds: Bounds<Pixels>,
}

/// A single-line text element painted with cached Gaussian-blurred glyphs.
///
/// The text uses GPUI's normal shaping, alignment, typography inheritance, and
/// layout. Blur affects painting only, so swapping `TextBlur` with ordinary
/// text does not move neighboring content.
pub struct TextBlur {
    text: SharedString,
    radius: Pixels,
    interactivity: Interactivity,
}

impl TextBlur {
    /// Creates blurred single-line text with a three-pixel blur radius.
    #[track_caller]
    pub fn new(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            radius: px(3.),
            interactivity: Interactivity::new(),
        }
    }

    /// Sets the Gaussian kernel radius in logical pixels.
    pub fn radius(mut self, radius: Pixels) -> Self {
        self.radius = radius.max(px(0.));
        self
    }
}

impl IntoElement for TextBlur {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Styled for TextBlur {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.interactivity.base_style
    }
}

impl InteractiveElement for TextBlur {
    fn interactivity(&mut self) -> &mut Interactivity {
        &mut self.interactivity
    }
}

impl Element for TextBlur {
    type RequestLayoutState = TextBlurLayout;
    type PrepaintState = Option<Hitbox>;

    fn id(&self) -> Option<ElementId> {
        self.interactivity.element_id.clone()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.interactivity.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        Some(accesskit::Role::Label)
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        node.set_value(self.text.to_string());
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let state = TextBlurLayout::default();
        let state_for_measure = state.clone();
        let text = self.text.clone();

        let layout_id = self.interactivity.request_layout(
            global_id,
            inspector_id,
            window,
            cx,
            |style, window, _cx| {
                window.with_text_style(style.text_style().cloned(), |window| {
                    let text_style = window.text_style();
                    let font_size = text_style.font_size.to_pixels(window.rem_size());
                    let line_height = window.pixel_snap(
                        text_style
                            .line_height
                            .to_pixels(font_size.into(), window.rem_size()),
                    );
                    let run = text_style.to_run(text.len());

                    window.request_measured_layout(style, move |_, _, window, _cx| {
                        let line = window.text_system().shape_line(
                            text.clone(),
                            font_size,
                            std::slice::from_ref(&run),
                            None,
                        );
                        let measured_size = size(line.width().ceil(), line_height);
                        state_for_measure
                            .0
                            .borrow_mut()
                            .replace(TextBlurLayoutInner {
                                line,
                                line_height,
                                content_bounds: Bounds::new(Point::default(), measured_size),
                            });
                        measured_size
                    })
                })
            },
        );

        (layout_id, state)
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let measured_size = state
            .0
            .borrow()
            .as_ref()
            .map(|layout| size(layout.line.width(), layout.line_height))
            .unwrap_or_default();

        self.interactivity.prepaint(
            global_id,
            inspector_id,
            bounds,
            measured_size,
            window,
            cx,
            |style, scroll_offset, hitbox, window, _cx| {
                let padding = style
                    .padding
                    .to_pixels(bounds.size.into(), window.rem_size());
                if let Some(layout) = state.0.borrow_mut().as_mut() {
                    layout.content_bounds = Bounds::new(
                        bounds.origin + point(padding.left, padding.top) + scroll_offset,
                        size(
                            (bounds.size.width - padding.left - padding.right).max(px(0.)),
                            (bounds.size.height - padding.top - padding.bottom).max(px(0.)),
                        ),
                    );
                }
                hitbox
            },
        )
    }

    fn paint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        hitbox: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.interactivity.paint(
            global_id,
            inspector_id,
            bounds,
            hitbox.as_ref(),
            window,
            cx,
            |_style, window, cx| {
                let state = state.0.borrow();
                let Some(layout) = state.as_ref() else {
                    return;
                };
                layout
                    .line
                    .paint_blurred(
                        layout.content_bounds.origin,
                        layout.line_height,
                        window.text_style().text_align,
                        Some(layout.content_bounds.size.width),
                        self.radius,
                        window,
                        cx,
                    )
                    .ok();
            },
        );
    }
}
