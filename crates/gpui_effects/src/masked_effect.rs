use gpui::{
    A11ySubtreeBuilder, AnyElement, App, Bounds, EffectShader, EffectUniforms, Element, ElementId,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement,
    Pixels, SharedString, StyleRefinement, Styled, Svg, Transformation, Window, div, svg,
};

/// Applies a custom mask shader to monochrome content painted by an element.
pub fn masked_effect<E>(element: E, shader: EffectShader) -> MaskedEffect<E::Element>
where
    E: IntoElement,
{
    MaskedEffect::new(element.into_element(), shader)
}

/// Creates a styled text container rendered through a custom mask shader.
pub fn effect_text(text: impl Into<SharedString>, shader: EffectShader) -> MaskedEffect<gpui::Div> {
    masked_effect(div().child(text.into()), shader)
}

/// Creates a monochrome SVG rendered through a custom mask shader.
pub fn effect_svg(path: impl Into<SharedString>, shader: EffectShader) -> MaskedEffect<Svg> {
    masked_effect(svg().path(path).text_color(gpui::white()), shader)
}

/// An element wrapper that evaluates a fragment shader through alpha masks.
pub struct MaskedEffect<E: Element> {
    element: E,
    shader: EffectShader,
    uniforms: EffectUniforms,
    time: f32,
    opacity: f32,
}

impl<E: Element> MaskedEffect<E> {
    /// Creates a masked effect around an element.
    pub fn new(element: E, shader: EffectShader) -> Self {
        assert!(
            shader.is_mask(),
            "masked effects require EffectShader::wgsl_mask"
        );
        Self {
            element,
            shader,
            uniforms: EffectUniforms::default(),
            time: 0.0,
            opacity: 1.0,
        }
    }

    /// Replaces all shader uniform slots.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.uniforms = uniforms;
        self
    }

    /// Sets one four-component shader uniform slot.
    pub fn uniform(mut self, index: usize, value: [f32; 4]) -> Self {
        self.uniforms.set_slot(index, value);
        self
    }

    /// Sets the animation time supplied to the shader.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Sets the opacity applied after mask coverage.
    pub fn effect_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Returns the shader backing this masked effect.
    pub fn shader(&self) -> &EffectShader {
        &self.shader
    }

    /// Returns the wrapped element.
    pub fn into_inner(self) -> E {
        self.element
    }
}

impl MaskedEffect<Svg> {
    /// Applies a GPU transformation to the masked SVG.
    pub fn with_transformation(mut self, transformation: Transformation) -> Self {
        self.element = self.element.with_transformation(transformation);
        self
    }
}

impl<E: Element> IntoElement for MaskedEffect<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: Element> Element for MaskedEffect<E> {
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
        let shader = self.shader.clone();
        let uniforms = self.uniforms;
        let time = self.time;
        let opacity = self.opacity;
        window.with_masked_effect(bounds, shader, uniforms, time, opacity, |window| {
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

impl<E> Styled for MaskedEffect<E>
where
    E: Element + Styled,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E> InteractiveElement for MaskedEffect<E>
where
    E: Element + InteractiveElement,
{
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.element.interactivity()
    }
}

impl<E> ParentElement for MaskedEffect<E>
where
    E: Element + ParentElement,
{
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}
