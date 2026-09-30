use gpui::{
    AnyElement, App, Bounds, EffectShader, EffectUniforms, Element, GlobalElementId,
    InspectorElementId, InteractiveElement, IntoElement, LayoutId, ParentElement, Pixels,
    PointerTransform, StyleRefinement, Styled, TransformationMatrix, Window,
};

/// Transforms a captured subtree and its interaction geometry with one matrix.
///
/// The matrix maps source to display in logical pixels relative to the element's
/// top-left corner. Layout stays unchanged. Both the source capture and displayed
/// result are clipped to the element's bounds. Deferred overlays remain unscaled;
/// use `anchored().map_anchor(true)` to attach them to transformed content.
/// Platforms without subtree effects retain ordinary drawing and input.
///
/// # Panics
/// Panics if the matrix is nonfinite, singular, or has an unrepresentable inverse.
pub fn transform_group<E: IntoElement>(
    element: E,
    matrix: TransformationMatrix,
) -> TransformGroup<E::Element> {
    assert!(
        matrix.inverse().is_some(),
        "transform_group requires a finite invertible matrix"
    );
    TransformGroup {
        element: Some(element.into_element()),
        matrix,
    }
}

/// A fixed-layout viewport whose captured content uses an affine transform.
///
/// Pointer events, IME geometry, accessibility and opt-in popup anchors share
/// the drawing matrix. Scroll deltas and keyboard focus order remain unchanged.
/// Captured pixels use the window's raster density; zoom does not rerasterize text
/// at a higher density or reveal content outside the source capture.
pub struct TransformGroup<E: Element> {
    element: Option<E>,
    matrix: TransformationMatrix,
}

fn window_matrix(matrix: TransformationMatrix, bounds: Bounds<Pixels>) -> TransformationMatrix {
    TransformationMatrix::unit()
        .translate(bounds.origin.scale(1.))
        .compose(matrix)
        .translate(bounds.origin.scale(-1.))
}

fn uniforms(matrix: TransformationMatrix, capture: Bounds<Pixels>, scale: f32) -> EffectUniforms {
    let inverse = matrix
        .inverse()
        .expect("resolved transform must be invertible");
    // Sampling is relative to the snapped capture, while the authored pivot is
    // the unsnapped layout origin. Keep that difference at fractional DPI.
    let translation = (inverse.apply(capture.origin) - capture.origin).scale(scale);
    let [[a, b], [c, d]] = inverse.rotation_scale;
    EffectUniforms::default()
        .with_slot(0, [a, b, translation.x.0, 0.])
        .with_slot(1, [c, d, translation.y.0, 0.])
}

/// Affine capture shader; two uniform slots contain inverse matrix rows in device pixels.
pub fn transform_group_shader() -> EffectShader {
    EffectShader::wgsl_image(include_str!("shaders/transform_group.wgsl"))
}

impl<E: Element> IntoElement for TransformGroup<E> {
    type Element = Self;
    fn into_element(self) -> Self::Element {
        self
    }
}

impl<E: Element> Element for TransformGroup<E> {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        // Retain a drawable child so its own accessibility node and synthetic
        // children are registered inside the transform scope as well.
        let mut child = self
            .element
            .take()
            .expect("layout requested twice")
            .into_any_element();
        (child.request_layout(window, cx), child)
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if !window.supports_subtree_effects() {
            child.prepaint(window, cx);
            return;
        }
        let transform = PointerTransform::affine(window_matrix(self.matrix, bounds))
            .expect("resolved transform must be invertible");
        window.prepaint_subtree_effect(|window| {
            window.with_pointer_transform(bounds, transform, |window| child.prepaint(window, cx))
        });
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !window.supports_subtree_effects() {
            child.paint(window, cx);
            return;
        }
        let matrix = window_matrix(self.matrix, bounds);
        let transform =
            PointerTransform::affine(matrix).expect("resolved transform must be invertible");
        let uniforms = uniforms(
            matrix,
            window.pixel_snap_bounds(bounds),
            window.scale_factor(),
        );
        window.with_subtree_effect(
            bounds,
            transform_group_shader(),
            uniforms,
            0.,
            1.,
            |window| {
                window.with_pointer_transform(bounds, transform, |window| child.paint(window, cx))
            },
        );
    }
}

impl<E: Element + Styled> Styled for TransformGroup<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element
            .as_mut()
            .expect("cannot style after layout")
            .style()
    }
}
impl<E: Element + InteractiveElement> InteractiveElement for TransformGroup<E> {
    fn interactivity(&mut self) -> &mut gpui::Interactivity {
        self.element
            .as_mut()
            .expect("cannot change interactivity after layout")
            .interactivity()
    }
}
impl<E: Element + ParentElement> ParentElement for TransformGroup<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element
            .as_mut()
            .expect("cannot add children after layout")
            .extend(elements);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod gpu_tests;
