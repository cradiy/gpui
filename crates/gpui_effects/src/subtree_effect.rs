use gpui::{
    A11ySubtreeBuilder, AnyElement, App, Bounds, EffectShader, EffectUniforms, Element, ElementId,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement,
    Pixels, StyleRefinement, Styled, Window, px,
};

/// Captures an element subtree and applies a single-image effect to its pixels.
pub fn subtree_effect<E>(element: E, shader: EffectShader) -> SubtreeEffect<E::Element>
where
    E: IntoElement,
{
    SubtreeEffect::new(element.into_element(), shader)
}

/// An element wrapper that composites its children through an offscreen texture.
///
/// Layout, accessibility and hit testing use the wrapped element's coordinates.
/// Shader displacement affects pixels only. Unsupported platforms draw the
/// wrapped element normally. Check `Window::supports_subtree_effects` for support.
pub struct SubtreeEffect<E: Element> {
    element: E,
    shader: EffectShader,
    uniforms: EffectUniforms,
    pixel_uniform_slots: [bool; gpui::EFFECT_UNIFORM_SLOTS],
    time: f32,
    opacity: f32,
    padding: Pixels,
    enabled: bool,
}

impl<E: Element> SubtreeEffect<E> {
    /// Creates a subtree effect using a shader made with `EffectShader::wgsl_image`.
    pub fn new(element: E, shader: EffectShader) -> Self {
        assert!(
            shader.image_count() == 1 && !shader.is_mask(),
            "subtree effects require a single-image shader"
        );
        Self {
            element,
            shader,
            uniforms: EffectUniforms::default(),
            pixel_uniform_slots: [false; gpui::EFFECT_UNIFORM_SLOTS],
            time: 0.0,
            opacity: 1.0,
            padding: px(0.),
            enabled: true,
        }
    }

    /// Replaces all shader uniform slots.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.uniforms = uniforms;
        self.pixel_uniform_slots.fill(false);
        self
    }

    /// Sets one four-component shader uniform slot.
    pub fn uniform(mut self, index: usize, value: [f32; 4]) -> Self {
        self.uniforms.set_slot(index, value);
        self.pixel_uniform_slots[index] = false;
        self
    }

    /// Sets a logical-pixel slot, converted to device pixels at paint time.
    pub fn uniform_pixels(mut self, index: usize, value: [Pixels; 4]) -> Self {
        self.uniforms.set_slot(index, value.map(f32::from));
        self.pixel_uniform_slots[index] = true;
        self
    }

    fn scaled_uniforms(&self, scale_factor: f32) -> EffectUniforms {
        let mut uniforms = self.uniforms;
        for (index, scale) in self.pixel_uniform_slots.iter().enumerate() {
            if *scale {
                uniforms.set_slot(
                    index,
                    self.uniforms.slots()[index].map(|v| v * scale_factor),
                );
            }
        }
        uniforms
    }

    /// Sets the animation time supplied to the shader.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Sets the opacity of the composited subtree.
    pub fn effect_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Reserves paint-only space around the element for shadows and displaced pixels.
    pub fn capture_padding(mut self, padding: Pixels) -> Self {
        self.padding = padding.max(px(0.));
        self
    }

    /// Enables capture. Disabled elements are painted directly.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Returns the shader used to composite the subtree.
    pub fn shader(&self) -> &EffectShader {
        &self.shader
    }

    /// Returns the wrapped element.
    pub fn into_inner(self) -> E {
        self.element
    }
}

impl<E: Element> IntoElement for SubtreeEffect<E> {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: Element> Element for SubtreeEffect<E> {
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
        if self.enabled {
            window.prepaint_subtree_effect(|window| {
                self.element
                    .prepaint(id, inspector_id, bounds, request_layout, window, cx)
            })
        } else {
            self.element
                .prepaint(id, inspector_id, bounds, request_layout, window, cx)
        }
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
        if !self.enabled {
            self.element.paint(
                id,
                inspector_id,
                bounds,
                request_layout,
                prepaint,
                window,
                cx,
            );
            return;
        }
        let shader = self.shader.clone();
        let uniforms = self.scaled_uniforms(window.scale_factor());
        let time = self.time;
        let opacity = self.opacity;
        window.with_subtree_effect(
            bounds.dilate(self.padding),
            shader,
            uniforms,
            time,
            opacity,
            |window| {
                self.element.paint(
                    id,
                    inspector_id,
                    bounds,
                    request_layout,
                    prepaint,
                    window,
                    cx,
                );
            },
        );
    }
}

impl<E> Styled for SubtreeEffect<E>
where
    E: Element + Styled,
{
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}

impl<E> InteractiveElement for SubtreeEffect<E>
where
    E: Element + InteractiveElement,
{
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.element.interactivity()
    }
}

impl<E> ParentElement for SubtreeEffect<E>
where
    E: Element + ParentElement,
{
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixel_uniforms_follow_scale_and_raw_overrides() {
        let effect = crate::subtree_identity(gpui::div())
            .uniform_pixels(0, [px(2.), px(4.), px(6.), px(8.)])
            .uniform(1, [0.25, 0.5, 0.75, 1.]);
        assert_eq!(effect.scaled_uniforms(2.).slots()[0], [4., 8., 12., 16.]);
        assert_eq!(effect.scaled_uniforms(1.5).slots()[0], [3., 6., 9., 12.]);
        assert_eq!(effect.scaled_uniforms(2.).slots()[1], [0.25, 0.5, 0.75, 1.]);

        let effect = effect.uniform(0, [1., 2., 3., 4.]);
        assert_eq!(effect.scaled_uniforms(2.).slots()[0], [1., 2., 3., 4.]);
        let effect = effect
            .uniform_pixels(0, [px(1.); 4])
            .uniforms(EffectUniforms::new().with_slot(0, [3.; 4]));
        assert_eq!(effect.scaled_uniforms(2.).slots()[0], [3.; 4]);
    }
}
