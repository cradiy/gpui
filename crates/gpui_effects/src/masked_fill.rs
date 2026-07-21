use gpui::{
    A11ySubtreeBuilder, App, Background, Bounds, Element, ElementId, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels,
    SharedString, StyleRefinement, Styled, Svg, Transformation, Window, div, svg,
};

/// Applies a GPU-evaluated fill to monochrome content painted by an element.
///
/// Text glyphs and monochrome SVGs contribute only their alpha coverage. Emoji,
/// images, and colored SVGs keep their original colors.
pub fn masked_fill<E>(element: E, background: impl Into<Background>) -> MaskedFill<E::Element>
where
    E: IntoElement,
{
    MaskedFill::new(element.into_element(), background)
}

/// Creates a styled text container whose glyphs are filled by `background`.
///
/// The returned element forwards styling to its inner `Div`, so text size,
/// weight, layout, and interaction methods remain available.
pub fn gradient_text(
    text: impl Into<SharedString>,
    background: impl Into<Background>,
) -> MaskedFill<gpui::Div> {
    masked_fill(div().child(text.into()), background)
}

/// Creates a monochrome SVG whose alpha mask is filled by `background`.
pub fn gradient_svg(
    path: impl Into<SharedString>,
    background: impl Into<Background>,
) -> MaskedFill<Svg> {
    // The existing monochrome SVG element uses text color as its opt-in paint
    // signal. The actual color is replaced by the masked fill during paint.
    masked_fill(svg().path(path).text_color(gpui::white()), background)
}

/// An element wrapper that replaces solid monochrome paint with a shared fill.
pub struct MaskedFill<E: Element> {
    element: E,
    background: Background,
}

impl<E: Element> MaskedFill<E> {
    /// Creates a masked-fill wrapper around an element.
    pub fn new(element: E, background: impl Into<Background>) -> Self {
        Self {
            element,
            background: background.into(),
        }
    }

    /// Replaces the fill sampled through the element's monochrome masks.
    pub fn fill(mut self, background: impl Into<Background>) -> Self {
        self.background = background.into();
        self
    }

    /// Advances a repeating gradient around its cycle.
    ///
    /// This is intended for use from GPUI's `with_animation` callback.
    pub fn phase(mut self, phase: f32) -> Self {
        self.background = self.background.phase(phase);
        self
    }

    /// Applies opacity to every color in the fill.
    pub fn fill_opacity(mut self, opacity: f32) -> Self {
        self.background = self.background.opacity(opacity.clamp(0.0, 1.0));
        self
    }

    /// Returns the wrapped element.
    pub fn into_inner(self) -> E {
        self.element
    }
}

impl MaskedFill<Svg> {
    /// Applies a GPU transformation to the masked SVG.
    pub fn with_transformation(mut self, transformation: Transformation) -> Self {
        self.element = self.element.with_transformation(transformation);
        self
    }
}

impl<E: Element> IntoElement for MaskedFill<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: Element> Element for MaskedFill<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;

    fn id(&self) -> Option<ElementId> {
        self.element.id()
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        self.element.source_location()
    }

    fn a11y_role(&self) -> Option<accesskit::Role> {
        self.element.a11y_role()
    }

    fn write_a11y_info(&self, node: &mut accesskit::Node) {
        self.element.write_a11y_info(node);
    }

    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        self.element.a11y_synthetic_children(prepaint, builder);
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.element.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.element
            .prepaint(id, inspector_id, bounds, request_layout, window, cx)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let background = self.background;
        window.with_masked_fill(bounds, background, |window| {
            self.element.paint(
                id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            );
        });
    }
}

impl<E> Styled for MaskedFill<E>
where
    E: Element + Styled,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E> InteractiveElement for MaskedFill<E>
where
    E: Element + InteractiveElement,
{
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.element.interactivity()
    }
}

impl<E> ParentElement for MaskedFill<E>
where
    E: Element + ParentElement,
{
    fn extend(&mut self, elements: impl IntoIterator<Item = gpui::AnyElement>) {
        self.element.extend(elements);
    }
}
