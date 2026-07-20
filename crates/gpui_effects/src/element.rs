use gpui::prelude::*;
use gpui::{
    App, Bounds, EffectShader, EffectUniforms, Element, ElementId, GlobalElementId, ImageSource,
    InspectorElementId, IntoElement, LayoutId, PaintEffect, Pixels, Style, StyleRefinement, Styled,
    Window,
};

/// Constructs a styled element backed by a custom fragment effect shader.
pub fn effect(shader: EffectShader) -> Effect {
    Effect::new(shader)
}

/// Constructs a styled effect that samples an image source.
pub fn image_effect(source: impl Into<ImageSource>, shader: EffectShader) -> Effect {
    Effect::new_image(source, shader)
}

/// Constructs a styled effect that samples separate primary and secondary images.
pub fn two_image_effect(
    primary: impl Into<ImageSource>,
    secondary: impl Into<ImageSource>,
    shader: EffectShader,
) -> Effect {
    Effect::new_two_images(primary, secondary, shader)
}

/// Constructs a styled effect that samples four independent images.
pub fn four_image_effect(
    first: impl Into<ImageSource>,
    second: impl Into<ImageSource>,
    third: impl Into<ImageSource>,
    fourth: impl Into<ImageSource>,
    shader: EffectShader,
) -> Effect {
    Effect::new_four_images(first, second, third, fourth, shader)
}

/// A styled rectangular element whose pixels are produced by an effect shader.
pub struct Effect {
    shader: EffectShader,
    uniforms: EffectUniforms,
    time: f32,
    opacity: f32,
    image_source: Option<ImageSource>,
    second_image_source: Option<ImageSource>,
    third_image_source: Option<ImageSource>,
    fourth_image_source: Option<ImageSource>,
    style: StyleRefinement,
}

impl Effect {
    /// Creates an effect element from a portable shader.
    pub fn new(shader: EffectShader) -> Self {
        Self {
            shader,
            uniforms: EffectUniforms::default(),
            time: 0.0,
            opacity: 1.0,
            image_source: None,
            second_image_source: None,
            third_image_source: None,
            fourth_image_source: None,
            style: StyleRefinement::default(),
        }
    }

    /// Creates an image-backed effect element.
    pub fn new_image(source: impl Into<ImageSource>, shader: EffectShader) -> Self {
        assert!(
            shader.uses_image(),
            "image effects require an EffectShader created with wgsl_image"
        );
        Self {
            image_source: Some(source.into()),
            ..Self::new(shader)
        }
    }

    /// Creates an effect backed by two independently sampled images.
    pub fn new_two_images(
        primary: impl Into<ImageSource>,
        secondary: impl Into<ImageSource>,
        shader: EffectShader,
    ) -> Self {
        assert_eq!(
            shader.image_count(),
            2,
            "two-image effects require EffectShader::wgsl_two_images"
        );
        Self {
            image_source: Some(primary.into()),
            second_image_source: Some(secondary.into()),
            ..Self::new(shader)
        }
    }

    /// Creates an effect backed by four independently sampled images.
    pub fn new_four_images(
        first: impl Into<ImageSource>,
        second: impl Into<ImageSource>,
        third: impl Into<ImageSource>,
        fourth: impl Into<ImageSource>,
        shader: EffectShader,
    ) -> Self {
        assert_eq!(
            shader.image_count(),
            4,
            "four-image effects require EffectShader::wgsl_four_images"
        );
        Self {
            image_source: Some(first.into()),
            second_image_source: Some(second.into()),
            third_image_source: Some(third.into()),
            fourth_image_source: Some(fourth.into()),
            ..Self::new(shader)
        }
    }

    /// Replaces all user-defined uniform slots.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.uniforms = uniforms;
        self
    }

    /// Sets one four-component user uniform slot.
    pub fn uniform(mut self, index: usize, value: [f32; 4]) -> Self {
        self.uniforms.set_slot(index, value);
        self
    }

    /// Sets the normalized animation time supplied to the shader.
    ///
    /// Built-in effects are periodic over `0.0..=1.0`, making them suitable
    /// for GPUI's repeating animation wrapper.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Sets the opacity applied to the shader result.
    pub fn effect_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Returns the shader backing this element.
    pub fn shader(&self) -> &EffectShader {
        &self.shader
    }
}

impl IntoElement for Effect {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for Effect {
    type RequestLayoutState = Style;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        if let Some(source) = &self.image_source {
            let _ = source.use_data(None, window, cx);
        }
        if let Some(source) = &self.second_image_source {
            let _ = source.use_data(None, window, cx);
        }
        if let Some(source) = &self.third_image_source {
            let _ = source.use_data(None, window, cx);
        }
        if let Some(source) = &self.fourth_image_source {
            let _ = source.use_data(None, window, cx);
        }
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let corner_radii = style
            .corner_radii
            .to_pixels(window.rem_size())
            .clamp_radii_for_quad_size(bounds.size);
        let effect = PaintEffect::new(bounds, self.shader.clone())
            .uniforms(self.uniforms)
            .time(self.time)
            .corner_radii(corner_radii)
            .opacity(self.opacity);

        let effect = if let Some(source) = &self.image_source {
            match source.use_data(None, window, cx) {
                Some(Ok(image)) if image.frame_count() > 0 => Some(effect.image(image, 0)),
                _ => None,
            }
        } else {
            Some(effect)
        };
        let effect = match (effect, self.second_image_source.as_ref()) {
            (Some(effect), Some(source)) => match source.use_data(None, window, cx) {
                Some(Ok(image)) if image.frame_count() > 0 => Some(effect.second_image(image, 0)),
                _ => None,
            },
            (effect, None) => effect,
            (None, Some(_)) => None,
        };
        let effect = match (effect, self.third_image_source.as_ref()) {
            (Some(effect), Some(source)) => match source.use_data(None, window, cx) {
                Some(Ok(image)) if image.frame_count() > 0 => Some(effect.third_image(image, 0)),
                _ => None,
            },
            (effect, None) => effect,
            (None, Some(_)) => None,
        };
        let effect = match (effect, self.fourth_image_source.as_ref()) {
            (Some(effect), Some(source)) => match source.use_data(None, window, cx) {
                Some(Ok(image)) if image.frame_count() > 0 => Some(effect.fourth_image(image, 0)),
                _ => None,
            },
            (effect, None) => effect,
            (None, Some(_)) => None,
        };

        style.paint(bounds, window, cx, move |window, _| {
            if let Some(effect) = effect {
                let _ = window.paint_effect(effect);
            }
        });
    }
}

impl Styled for Effect {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
