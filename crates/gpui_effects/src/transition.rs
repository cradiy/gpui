use gpui::{
    AnyElement, App, Bounds, ContentMask, EffectShader, EffectUniforms, Element, ElementId,
    GlobalElementId, HitboxBehavior, InspectorElementId, IntoElement, LayoutId, Pixels, Style,
    StyleRefinement, Styled, SubtreeInput, Window, div, prelude::*, px,
};

/// Two-content transition appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TransitionKind {
    /// Blur outgoing content while bringing incoming content into focus.
    #[default]
    BlurFade,
    /// Interpolate premultiplied colors without spatial filtering.
    CrossFade,
    /// Reveal incoming content from left to right through a soft edge.
    WipeRight,
    /// Reveal incoming content from right to left through a soft edge.
    WipeLeft,
    /// Reveal incoming content from top to bottom through a soft edge.
    WipeDown,
    /// Reveal incoming content from bottom to top through a soft edge.
    WipeUp,
}

/// Creates a transition between two independently laid-out element subtrees.
/// Set an explicit size or fill a bounded parent. Progress zero shows `from`;
/// progress one shows `to`. The caller owns animation time and focus management.
pub fn subtree_transition(
    id: impl Into<ElementId>,
    from: impl IntoElement,
    to: impl IntoElement,
) -> SubtreeTransition {
    SubtreeTransition {
        id: id.into(),
        inputs: [
            div()
                .id("from")
                .absolute()
                .inset_0()
                .size_full()
                .child(from)
                .into_any_element(),
            div()
                .id("to")
                .absolute()
                .inset_0()
                .size_full()
                .child(to)
                .into_any_element(),
        ],
        progress: 0.,
        kind: TransitionKind::default(),
        blur_radius: px(8.),
        edge_softness: 0.16,
        style: StyleRefinement::default(),
    }
}

/// Caller-controlled transition over two live subtrees. Intermediate frames block
/// ordinary pointer hit testing inside the region; endpoints expose only the visible input.
pub struct SubtreeTransition {
    id: ElementId,
    inputs: [AnyElement; 2],
    progress: f32,
    kind: TransitionKind,
    blur_radius: Pixels,
    edge_softness: f32,
    style: StyleRefinement,
}

impl SubtreeTransition {
    /// Sets progress in 0..=1. Non-finite values select the first input.
    pub fn progress(mut self, progress: f32) -> Self {
        self.progress = if progress.is_finite() {
            progress.clamp(0., 1.)
        } else {
            0.
        };
        self
    }

    /// Selects the built-in transition shader.
    pub fn kind(mut self, kind: TransitionKind) -> Self {
        self.kind = kind;
        self
    }

    /// Maximum blur support radius in logical pixels, clamped to 0..=24.
    pub fn blur_radius(mut self, radius: Pixels) -> Self {
        let radius = f32::from(radius);
        self.blur_radius = px(if radius.is_finite() {
            radius.clamp(0., 24.)
        } else {
            8.
        });
        self
    }

    /// Soft wipe edge width relative to the capture dimension, clamped to 0.001..=0.5.
    pub fn edge_softness(mut self, softness: f32) -> Self {
        self.edge_softness = if softness.is_finite() {
            softness.clamp(0.001, 0.5)
        } else {
            0.16
        };
        self
    }

    fn active_input(&self, window: &Window) -> Option<usize> {
        if self.progress == 0. {
            Some(0)
        } else if self.progress == 1. {
            Some(1)
        } else if !window.supports_subtree_effects() {
            Some(usize::from(self.progress >= 0.5))
        } else {
            None
        }
    }
}

/// Two-image transition shader. Slot 0: `[progress, blur_device_px, edge_softness, 0]`;
/// slot 1: `[wipe_axis_x, wipe_axis_y, 0, 0]`.
pub fn transition_shader(kind: TransitionKind) -> EffectShader {
    let source = match kind {
        TransitionKind::BlurFade => include_str!("shaders/transition_blur.wgsl"),
        TransitionKind::CrossFade => include_str!("shaders/transition_crossfade.wgsl"),
        _ => include_str!("shaders/transition_wipe.wgsl"),
    };
    EffectShader::wgsl_two_images(format!(
        "{}\n{source}",
        include_str!("shaders/transition_common.wgsl")
    ))
}

impl IntoElement for SubtreeTransition {
    type Element = gpui::Stateful<gpui::Div>;
    fn into_element(mut self) -> Self::Element {
        let style = std::mem::take(&mut self.style);
        let mut container = div()
            .id(self.id.clone())
            .relative()
            .child(TransitionContent(self));
        container.style().refine(&style);
        container
    }
}
impl Styled for SubtreeTransition {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

struct TransitionContent(SubtreeTransition);

impl IntoElement for TransitionContent {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for TransitionContent {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        Some("content".into())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let style = Style {
            size: gpui::size(gpui::relative(1.).into(), gpui::relative(1.).into()),
            ..Default::default()
        };
        let children = if let Some(index) = self.0.active_input(window) {
            vec![self.0.inputs[index].request_layout(window, cx)]
        } else {
            self.0
                .inputs
                .iter_mut()
                .map(|input| input.request_layout(window, cx))
                .collect()
        };
        (window.request_layout(style, children, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(index) = self.0.active_input(window) {
                self.0.inputs[index].prepaint(window, cx);
            } else {
                window.prepaint_subtree_effect(|window| {
                    for input in &mut self.0.inputs {
                        input.prepaint(window, cx);
                    }
                });
                window.insert_hitbox(bounds, HitboxBehavior::BlockMouse);
            }
        });
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(index) = self.0.active_input(window) {
                self.0.inputs[index].paint(window, cx);
                return;
            }
            let axis = match self.0.kind {
                TransitionKind::WipeLeft => [-1., 0., 0., 0.],
                TransitionKind::WipeUp => [0., -1., 0., 0.],
                TransitionKind::WipeDown => [0., 1., 0., 0.],
                _ => [1., 0., 0., 0.],
            };
            let uniforms = EffectUniforms::new()
                .with_slot(
                    0,
                    [
                        self.0.progress,
                        f32::from(self.0.blur_radius) * window.scale_factor(),
                        self.0.edge_softness,
                        0.,
                    ],
                )
                .with_slot(1, axis);
            window.with_subtree_pair(
                bounds,
                transition_shader(self.0.kind),
                uniforms,
                0.,
                1.,
                |input, window| {
                    self.0.inputs[usize::from(input == SubtreeInput::Second)].paint(window, cx);
                },
            );
        });
    }
}
