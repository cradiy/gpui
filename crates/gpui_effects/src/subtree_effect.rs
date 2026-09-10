use crate::EffectStage;
use gpui::{
    A11ySubtreeBuilder, AnyElement, App, Bounds, EffectShader, EffectUniforms, Element, ElementId,
    GlobalElementId, InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement,
    Pixels, StyleRefinement, Styled, Window, px,
};

/// Captures content once and applies enabled stages in iteration order.
/// An empty or fully disabled chain paints the content directly.
pub fn subtree_effect_chain<E: IntoElement>(
    element: E,
    stages: impl IntoIterator<Item = EffectStage>,
) -> SubtreeEffect<E::Element> {
    let mut stages = stages.into_iter().filter(|stage| stage.enabled);
    let first = stages.next();
    let mut effect = SubtreeEffect::new(
        element.into_element(),
        first
            .as_ref()
            .filter(|stage| stage.images.0.is_empty())
            .map(|stage| stage.shader.clone())
            .unwrap_or_else(crate::subtree_identity_shader),
    );
    effect.first_stage_enabled = first.is_some();
    if let Some(stage) = first {
        effect.pointer_transform = stage.pointer_transform;
        effect.shader = stage.shader;
        effect.images = stage.images;
        effect.uniforms = stage.uniforms;
        effect.pixel_uniform_slots = stage.pixel_uniform_slots;
        effect.padding = stage.padding;
        effect.bloom = stage.bloom;
        effect.feedback = stage.feedback;
        effect.distance_field = stage.distance_field;
        effect.particles = stage.particles;
        effect.particle_transition = stage.particle_transition;
    }
    for stage in stages {
        effect = effect.then(stage);
    }
    effect
}

/// Captures an element subtree and applies a single-image effect to its pixels.
pub fn subtree_effect<E>(element: E, shader: EffectShader) -> SubtreeEffect<E::Element>
where
    E: IntoElement,
{
    SubtreeEffect::new(element.into_element(), shader)
}

/// An element wrapper that composites its children through an offscreen texture.
///
/// Layout and accessibility use the wrapped element's coordinates. Pointer mapping
/// is opt-in through [`Self::map_interaction`]. Unsupported platforms draw the
/// wrapped element normally. Check `Window::supports_subtree_effects` for support.
pub struct SubtreeEffect<E: Element> {
    pointer_transform: Option<gpui::PointerTransform>,
    map_interaction: bool,
    images: crate::effect_stage::StageImages,
    element: E,
    shader: EffectShader,
    uniforms: EffectUniforms,
    pixel_uniform_slots: [bool; gpui::EFFECT_UNIFORM_SLOTS],
    time: f32,
    opacity: f32,
    padding: Pixels,
    enabled: bool,
    first_stage_enabled: bool,
    bloom: Option<gpui::SubtreeBloomPass>,
    feedback: Option<gpui::SubtreeFeedbackPass>,
    distance_field: Option<gpui::SubtreeDistanceFieldPass>,
    particles: Option<gpui::SubtreeParticlePass>,
    particle_transition: Option<gpui::SubtreeParticleTransitionPass>,
    following_stages: Vec<EffectStage>,
}

impl<E: Element> SubtreeEffect<E> {
    /// Creates a subtree effect using a shader made with `EffectShader::wgsl_image`.
    pub fn new(element: E, shader: EffectShader) -> Self {
        assert!(
            shader.image_count() == 1 && !shader.is_mask(),
            "subtree effects require a single-image shader"
        );
        Self {
            pointer_transform: None,
            map_interaction: false,
            images: Default::default(),
            element,
            shader,
            uniforms: EffectUniforms::default(),
            pixel_uniform_slots: [false; gpui::EFFECT_UNIFORM_SLOTS],
            time: 0.0,
            opacity: 1.0,
            padding: px(0.),
            enabled: true,
            first_stage_enabled: true,
            bloom: None,
            feedback: None,
            distance_field: None,
            particles: None,
            particle_transition: None,
            following_stages: Vec::new(),
        }
    }

    /// Replaces all uniform slots of the first stage and clears its pointer transform.
    pub fn uniforms(mut self, uniforms: EffectUniforms) -> Self {
        self.pointer_transform = None;
        self.uniforms = uniforms;
        self.pixel_uniform_slots.fill(false);
        self
    }

    /// Sets one four-component uniform slot of the first stage and clears its pointer transform.
    pub fn uniform(mut self, index: usize, value: [f32; 4]) -> Self {
        self.pointer_transform = None;
        self.uniforms.set_slot(index, value);
        self.pixel_uniform_slots[index] = false;
        self
    }

    /// Sets a logical-pixel slot of the first stage, converted at paint time.
    /// Clears its pointer transform.
    pub fn uniform_pixels(mut self, index: usize, value: [Pixels; 4]) -> Self {
        self.pointer_transform = None;
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

    /// Sets the animation time supplied to every stage.
    /// Custom pointer transforms must use the same clock as their shader.
    pub fn time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Aligns pointer hit testing and event positions with the effect chain.
    /// Disabled by default. Keyboard focus and layout are unchanged.
    ///
    /// # Panics
    /// Rendering on a supported backend panics if an enabled stage lacks a pointer
    /// transform or uses external images.
    pub fn map_interaction(mut self, enabled: bool) -> Self {
        self.map_interaction = enabled;
        self
    }

    fn interaction_transform(&self) -> Option<gpui::PointerTransform> {
        if !self.enabled || !self.map_interaction {
            return None;
        }
        let mut transforms = Vec::new();
        if self.first_stage_enabled {
            assert!(
                self.images.0.is_empty(),
                "external image stages do not support interaction mapping"
            );
            transforms.push(
                self.pointer_transform
                    .clone()
                    .expect("interaction mapping requires a transform for each enabled stage"),
            );
        }
        for stage in &self.following_stages {
            assert!(
                stage.images.0.is_empty(),
                "external image stages do not support interaction mapping"
            );
            transforms.push(
                stage
                    .pointer_transform
                    .clone()
                    .expect("interaction mapping requires a transform for each enabled stage"),
            );
        }
        if transforms.is_empty() {
            return None;
        }
        Some(gpui::PointerTransform::chain(transforms))
    }

    /// Appends a stage without capturing the element again.
    pub fn then(mut self, stage: EffectStage) -> Self {
        if stage.enabled {
            self.padding += stage.padding;
            self.following_stages.push(stage);
        }
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

    /// Returns the first stage's shader, or identity for an empty chain.
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
        if self.enabled {
            self.images.preload(window, cx);
            for stage in &self.following_stages {
                stage.images.preload(window, cx);
            }
        }
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
        let transform = window
            .supports_subtree_effects()
            .then(|| self.interaction_transform())
            .flatten();
        if self.enabled && (self.first_stage_enabled || !self.following_stages.is_empty()) {
            window.prepaint_subtree_effect(|window| {
                let mut paint = |window: &mut Window| {
                    self.element
                        .prepaint(id, inspector_id, bounds, request_layout, window, cx)
                };
                if let Some(transform) = transform {
                    window.with_pointer_transform(bounds.dilate(self.padding), transform, paint)
                } else {
                    paint(window)
                }
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
        let mut passes = smallvec::SmallVec::<[gpui::SubtreeEffectPass; 2]>::new();
        if self.first_stage_enabled
            && let Some(images) = self.images.prepare(window, cx)
        {
            passes.push(gpui::SubtreeEffectPass {
                images,
                shader: self.shader.clone(),
                uniforms: self.scaled_uniforms(window.scale_factor()),
                time: self.time,
                bloom: self.bloom.clone(),
                distance_field: self.distance_field.clone(),
                particle_transition: self.particle_transition.map(|mut transition| {
                    transition.scale_factor = window.scale_factor();
                    transition
                }),
                particles: self.particles.clone().map(|mut particles| {
                    particles.scale_factor = window.scale_factor();
                    particles
                }),
                feedback: self.feedback.clone().map(|mut feedback| {
                    feedback.scale_factor = window.scale_factor();
                    feedback
                }),
            });
        }
        for stage in &self.following_stages {
            if let Some(images) = stage.images.prepare(window, cx) {
                let mut pass = stage.prepare(window.scale_factor(), self.time);
                pass.images = images;
                passes.push(pass);
            }
        }
        let opacity = self.opacity;
        if window.supports_subtree_effects()
            && opacity > 0.
            && bounds
                .dilate(self.padding)
                .intersects(&window.content_mask().bounds)
            && passes.iter().any(|pass| {
                pass.particles
                    .as_ref()
                    .is_some_and(|particles| particles.frame.needs_animation)
                    || pass
                        .feedback
                        .as_ref()
                        .is_some_and(|feedback| feedback.needs_animation)
            })
        {
            window.request_animation_frame();
        }
        let transform = window
            .supports_subtree_effects()
            .then(|| self.interaction_transform())
            .flatten();
        window.with_subtree_effect_chain(bounds.dilate(self.padding), &passes, opacity, |window| {
            let mut paint = |window: &mut Window| {
                self.element.paint(
                    id,
                    inspector_id,
                    bounds,
                    request_layout,
                    prepaint,
                    window,
                    cx,
                )
            };
            if let Some(transform) = transform {
                window.with_pointer_transform(bounds.dilate(self.padding), transform, paint)
            } else {
                paint(window)
            }
        });
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
    fn bloom_and_color_preserve_pointer_mapping_across_stage_toggles() {
        use crate::{BloomOptions, SubtreeColorOptions};
        use gpui::{PointerTransform, point, size};

        for bloom in [false, true] {
            for color in [false, true] {
                for scale in [1., 1.5, 2.] {
                    let effect = subtree_effect_chain(
                        gpui::div(),
                        [
                            EffectStage::identity().pointer_transform(PointerTransform::new(
                                |p, _, _| p - point(px(12.), px(8.)),
                            )),
                            EffectStage::bloom(BloomOptions::default()).enabled(bloom),
                            EffectStage::color_adjust(SubtreeColorOptions {
                                saturation: 0.,
                                ..Default::default()
                            })
                            .enabled(color),
                        ],
                    )
                    .map_interaction(true);
                    let bounds = Bounds::new(point(px(40.), px(60.)), size(px(200.), px(120.)))
                        .dilate(effect.padding);
                    let mapping = effect.interaction_transform().unwrap();
                    for p in [point(px(110.), px(95.)), point(px(330.), px(220.))] {
                        assert_eq!(mapping.map(p, bounds, scale), p - point(px(12.), px(8.)));
                    }
                }
            }
        }
    }

    #[test]
    fn pointer_mapping_reverses_only_enabled_stages() {
        use gpui::{PointerTransform, point, size};
        let translate = || {
            EffectStage::identity()
                .pointer_transform(PointerTransform::new(|p, _, _| p - point(px(20.), px(0.))))
        };
        let effect = subtree_effect_chain(
            gpui::div(),
            [
                translate(),
                translate().enabled(false),
                EffectStage::identity().pointer_transform(PointerTransform::new(|p, _, _| p / 2.)),
            ],
        )
        .map_interaction(true);
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(200.), px(100.)));
        assert_eq!(
            effect
                .interaction_transform()
                .unwrap()
                .map(point(px(100.), px(40.)), bounds, 1.),
            point(px(30.), px(20.))
        );
        assert!(effect.enabled(false).interaction_transform().is_none());
    }

    #[test]
    fn chains_omit_disabled_stages_and_accumulate_padding() {
        let effect = subtree_effect_chain(
            gpui::div(),
            [
                EffectStage::blur(px(40.)).enabled(false),
                EffectStage::blur(px(2.)),
                EffectStage::wave(Default::default()),
            ],
        )
        .then(EffectStage::blur(px(90.)).enabled(false));
        assert_eq!(effect.shader().id(), crate::subtree_blur_shader().id());
        assert_eq!(effect.following_stages.len(), 1);
        assert_eq!(effect.padding, px(9.));
        let wave = effect.following_stages[0].prepare(1.5, 2.5);
        assert_eq!(wave.uniforms.slots()[0], [9., 270., 0., 0.]);
        assert_eq!(wave.uniforms.slots()[1][0], 1.5);
        assert_eq!(wave.time, 2.5);
        let effect = effect
            .capture_padding(px(20.))
            .then(EffectStage::blur(px(3.)));
        assert_eq!(effect.padding, px(23.));

        let empty = subtree_effect_chain(gpui::div(), []);
        assert!(!empty.first_stage_enabled && empty.following_stages.is_empty());
        let disabled =
            subtree_effect_chain(gpui::div(), [EffectStage::blur(px(40.)).enabled(false)]);
        assert!(!disabled.first_stage_enabled && disabled.following_stages.is_empty());
        assert_eq!(disabled.padding, px(0.));

        for options in [
            crate::BloomOptions {
                intensity: 0.,
                ..Default::default()
            },
            crate::BloomOptions {
                radius: px(0.),
                ..Default::default()
            },
        ] {
            let disabled = crate::subtree_bloom(gpui::div(), options);
            assert!(!disabled.first_stage_enabled && disabled.following_stages.is_empty());
            assert_eq!(disabled.padding, px(0.));
        }
    }

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
