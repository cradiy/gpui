use crate::{Bounds, Pixels, Point, TransformationMatrix};
use std::{fmt, rc::Rc};

/// Maps a displayed point back to source coordinates within a paint region.
#[derive(Clone)]
pub struct PointerTransform(TransformKind);

type MapPosition = dyn Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Point<Pixels>;
type HitPosition = dyn Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Option<Point<Pixels>>;

#[derive(Clone)]
enum TransformKind {
    Noninteractive,
    Affine {
        forward: TransformationMatrix,
        inverse: TransformationMatrix,
    },
    Function(Rc<MapPosition>),
    Projection {
        map: Rc<MapPosition>,
        hit: Rc<HitPosition>,
    },
    Chain(Rc<[PointerTransform]>),
}

impl PointerTransform {
    /// Rejects pointer hits inside a decorative captured subtree without occluding its ancestors.
    pub fn noninteractive() -> Self {
        Self(TransformKind::Noninteractive)
    }
    /// The callback receives window-relative coordinates, snapped capture bounds,
    /// and the logical-to-device scale factor.
    pub fn new(
        map: impl Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Point<Pixels> + 'static,
    ) -> Self {
        Self(TransformKind::Function(Rc::new(map)))
    }

    /// Maps into a separate source coordinate space with explicit visibility testing.
    /// `hit` returns `None` for occluded or missing surfaces. `map` also handles
    /// positions outside the displayed bounds so captured drags can continue.
    /// The caller validates source bounds; ancestor display clipping still applies.
    pub fn projected(
        map: impl Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Point<Pixels> + 'static,
        hit: impl Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Option<Point<Pixels>> + 'static,
    ) -> Self {
        Self(TransformKind::Projection {
            map: Rc::new(map),
            hit: Rc::new(hit),
        })
    }

    /// Preserves pointer coordinates.
    pub fn identity() -> Self {
        Self(TransformKind::Affine {
            forward: TransformationMatrix::unit(),
            inverse: TransformationMatrix::unit(),
        })
    }

    /// Uses a source-to-display matrix in logical window pixels for inverse pointer mapping.
    /// The matrix is independent of display density and is compared by value for view caching.
    /// Returns `None` for nonfinite, singular or unrepresentable inverse matrices.
    /// This changes input coordinates only; the caller supplies matching drawing transforms.
    pub fn affine(source_to_display: TransformationMatrix) -> Option<Self> {
        Some(Self(TransformKind::Affine {
            forward: source_to_display,
            inverse: source_to_display.inverse()?,
        }))
    }

    /// Maps source coordinates to display coordinates for affine transforms and affine chains.
    /// Arbitrary callbacks and decorative transforms do not provide a forward mapping.
    pub fn source_to_display(&self, position: Point<Pixels>) -> Option<Point<Pixels>> {
        match &self.0 {
            TransformKind::Affine { forward, .. } => Some(forward.apply(position)),
            TransformKind::Chain(transforms) => {
                transforms.iter().try_fold(position, |point, transform| {
                    transform.source_to_display(point)
                })
            }
            _ => None,
        }
    }

    fn cache_eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (TransformKind::Noninteractive, TransformKind::Noninteractive) => true,
            (
                TransformKind::Affine { forward: a, .. },
                TransformKind::Affine { forward: b, .. },
            ) => a == b,
            (TransformKind::Chain(a), TransformKind::Chain(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(a, b)| a.cache_eq(b))
            }
            // Even a shared callback may capture mutable state. New scopes must invalidate.
            _ => false,
        }
    }

    /// Composes transforms supplied in paint-pass order. Pointer mapping runs in reverse;
    /// hit testing rejects samples outside the capture at any intermediate stage.
    pub fn chain(transforms: impl IntoIterator<Item = Self>) -> Self {
        Self(TransformKind::Chain(transforms.into_iter().collect()))
    }

    /// Evaluates the inverse mapping. Coordinates remain window-relative.
    pub fn map(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
    ) -> Point<Pixels> {
        match &self.0 {
            TransformKind::Noninteractive => position,
            TransformKind::Affine { inverse, .. } => inverse.apply(position),
            TransformKind::Function(map) => map(position, bounds, scale_factor),
            TransformKind::Projection { map, .. } => map(position, bounds, scale_factor),
            TransformKind::Chain(transforms) => {
                transforms.iter().rev().fold(position, |p, transform| {
                    transform.map(p, bounds, scale_factor)
                })
            }
        }
    }

    fn hit_position(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        scale_factor: f32,
    ) -> Option<Point<Pixels>> {
        match &self.0 {
            TransformKind::Noninteractive => None,
            TransformKind::Affine { inverse, .. } => {
                let source = inverse.apply(position);
                bounds.contains(&source).then_some(source)
            }
            TransformKind::Projection { hit, .. } => hit(position, bounds, scale_factor),
            TransformKind::Function(map) => {
                let source = map(position, bounds, scale_factor);
                bounds.contains(&source).then_some(source)
            }
            TransformKind::Chain(transforms) => {
                transforms.iter().rev().try_fold(position, |p, transform| {
                    transform.hit_position(p, bounds, scale_factor)
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{point, px, size};

    #[test]
    fn decorative_capture_rejects_hits_without_changing_parent_mapping() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
        let parent = PointerMapping::default();
        let child = parent.then(bounds, bounds, 1., PointerTransform::noninteractive());
        let p = point(px(50.), px(50.));
        assert_eq!(parent.hit_position(p), Some(p));
        assert_eq!(child.hit_position(p), None);
        assert_eq!(child.map(p), p);
    }

    #[test]
    fn nested_mapping_orders_transforms_and_clips_before_sampling() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(200.)));
        let outer = PointerMapping::default().then(
            bounds,
            bounds,
            1.,
            PointerTransform::new(|p, _, _| p / 2.),
        );
        let inner = outer.then(
            bounds,
            Bounds::new(point(px(100.), px(0.)), size(px(100.), px(200.))),
            1.,
            PointerTransform::new(|p, _, _| p - point(px(100.), px(0.))),
        );
        assert_eq!(
            inner.hit_position(point(px(260.), px(80.))),
            Some(point(px(30.), px(40.)))
        );
        assert_eq!(inner.hit_position(point(px(80.), px(80.))), None);
        assert_eq!(
            inner.map(point(px(500.), px(80.))),
            point(px(150.), px(40.))
        );
        assert_eq!(inner.hit_position(point(px(500.), px(80.))), None);
    }

    #[test]
    fn chain_clipping_preserves_transparent_intermediate_samples() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
        let mapping = PointerMapping::default().then(
            bounds,
            bounds,
            1.,
            PointerTransform::chain([
                PointerTransform::new(|p, _, _| p - point(px(200.), px(0.))),
                PointerTransform::new(|p, _, _| p + point(px(200.), px(0.))),
            ]),
        );
        let p = point(px(20.), px(20.));
        assert_eq!(mapping.map(p), p);
        assert_eq!(mapping.hit_position(p), None);
    }

    #[test]
    fn affine_scopes_compose_invert_and_keep_capture_clipping() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(200.)));
        let scale = PointerTransform::affine(TransformationMatrix {
            rotation_scale: [[2., 0.], [0., 2.]],
            translation: [0., 0.],
        })
        .unwrap();
        let translate = PointerTransform::affine(TransformationMatrix {
            translation: [100., 0.],
            ..TransformationMatrix::unit()
        })
        .unwrap();
        let nested = PointerMapping::default()
            .then(bounds, bounds, 2., scale.clone())
            .then(
                bounds,
                Bounds::new(point(px(100.), px(0.)), size(px(100.), px(200.))),
                2.,
                translate.clone(),
            );
        let display = point(px(260.), px(80.));
        let source = point(px(30.), px(40.));
        assert_eq!(nested.hit_position(display), Some(source));
        assert_eq!(nested.source_to_display(source), Some(display));
        assert_eq!(nested.hit_position(point(px(80.), px(80.))), None);
        assert_eq!(
            nested.map(point(px(500.), px(80.))),
            point(px(150.), px(40.))
        );
        assert_eq!(nested.hit_position(point(px(500.), px(80.))), None);
        let chain = PointerTransform::chain([translate, scale]);
        assert_eq!(chain.map(display, bounds, 1.), source);
        assert_eq!(chain.source_to_display(source), Some(display));
    }

    #[test]
    fn affine_cache_identity_includes_scope_and_excludes_mutable_callbacks() {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(100.), px(100.)));
        let make = |offset| {
            PointerTransform::affine(TransformationMatrix {
                translation: [offset, 0.],
                ..TransformationMatrix::unit()
            })
            .unwrap()
        };
        let scope = |clip, density, transform| {
            PointerMapping::default().then(bounds, clip, density, transform)
        };
        let original = scope(bounds, 1., make(10.));
        assert_eq!(original, scope(bounds, 1., make(10.)));
        assert_ne!(original, scope(bounds, 1., make(20.)));
        assert_ne!(original, scope(bounds, 2., make(10.)));
        assert_ne!(original, scope(bounds.dilate(px(-5.)), 1., make(10.)));
        let offset = Rc::new(std::cell::Cell::new(px(0.)));
        let transform = PointerTransform::new(move |p, _, _| p - point(offset.get(), px(0.)));
        assert_ne!(
            scope(bounds, 1., transform.clone()),
            scope(bounds, 1., transform)
        );
        assert_eq!(
            scope(bounds, 1., PointerTransform::identity()),
            scope(bounds, 1., PointerTransform::identity())
        );
    }

    #[test]
    fn affine_inverse_rejects_invalid_values_and_handles_shear_and_reflection() {
        for matrix in [
            TransformationMatrix {
                rotation_scale: [[1., 2.], [2., 4.]],
                translation: [0., 0.],
            },
            TransformationMatrix {
                translation: [f32::NAN, 0.],
                ..TransformationMatrix::unit()
            },
            TransformationMatrix {
                rotation_scale: [[f32::INFINITY, 0.], [0., 1.]],
                ..TransformationMatrix::unit()
            },
            TransformationMatrix {
                rotation_scale: [[f32::from_bits(1), 0.], [0., 1.]],
                ..TransformationMatrix::unit()
            },
        ] {
            assert!(PointerTransform::affine(matrix).is_none());
        }
        let matrix = TransformationMatrix {
            rotation_scale: [[-2., 1.], [0., 4.]],
            translation: [30., -10.],
        };
        let transform = PointerTransform::affine(matrix).unwrap();
        let source = point(px(3.), px(7.));
        let display = point(px(31.), px(18.));
        assert_eq!(transform.source_to_display(source), Some(display));
        assert_eq!(transform.map(display, Bounds::default(), 1.5), source);
    }

    #[test]
    fn affine_text_bounds_enclose_all_four_transformed_corners() {
        let bounds = Bounds::new(point(px(10.), px(20.)), size(px(3.), px(4.)));
        let transform = PointerTransform::affine(TransformationMatrix {
            rotation_scale: [[1., -1.], [1., 1.]],
            translation: [5., 7.],
        })
        .unwrap();
        let mapping = PointerMapping::default().then(bounds, bounds, 2., transform);
        assert_eq!(
            mapping.bounds_to_display(bounds),
            Some(Bounds::new(point(px(-9.), px(37.)), size(px(7.), px(7.))))
        );
    }
}

impl fmt::Debug for PointerTransform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PointerTransform")
    }
}

/// A nested pointer-coordinate scope retained by hitboxes and event listeners.
#[derive(Clone, Default, Debug)]
pub struct PointerMapping(Option<Rc<MappingNode>>);

#[derive(Debug)]
struct MappingNode {
    parent: PointerMapping,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    scale_factor: f32,
    transform: PointerTransform,
}

impl PartialEq for PointerMapping {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                Rc::ptr_eq(a, b)
                    || (a.bounds == b.bounds
                        && a.clip == b.clip
                        && a.scale_factor == b.scale_factor
                        && a.transform.cache_eq(&b.transform)
                        && a.parent == b.parent)
            }
            _ => false,
        }
    }
}

impl PointerMapping {
    pub(crate) fn is_identity(&self) -> bool {
        self.0.is_none()
    }
    pub(crate) fn then(
        &self,
        bounds: Bounds<Pixels>,
        clip: Bounds<Pixels>,
        scale_factor: f32,
        transform: PointerTransform,
    ) -> Self {
        Self(Some(Rc::new(MappingNode {
            parent: self.clone(),
            bounds,
            clip,
            scale_factor,
            transform,
        })))
    }

    /// Maps a point without clipping, so captured drags can continue outside the region.
    pub fn map(&self, position: Point<Pixels>) -> Point<Pixels> {
        match &self.0 {
            Some(node) => {
                node.transform
                    .map(node.parent.map(position), node.bounds, node.scale_factor)
            }
            None => position,
        }
    }

    /// Maps source coordinates back through nested affine scopes, without clipping.
    /// Returns `None` if any scope uses an arbitrary callback or decorative transform.
    pub fn source_to_display(&self, position: Point<Pixels>) -> Option<Point<Pixels>> {
        match &self.0 {
            Some(node) => node
                .parent
                .source_to_display(node.transform.source_to_display(position)?),
            None => Some(position),
        }
    }

    pub(crate) fn bounds_to_display(&self, bounds: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        let corners = [
            bounds.origin,
            bounds.top_right(),
            bounds.bottom_left(),
            bounds.bottom_right(),
        ];
        let mut min = self.source_to_display(bounds.origin)?;
        let mut max = min;
        for corner in corners {
            let point = self.source_to_display(corner)?;
            if !f32::from(point.x).is_finite() || !f32::from(point.y).is_finite() {
                return None;
            }
            min.x = min.x.min(point.x);
            min.y = min.y.min(point.y);
            max.x = max.x.max(point.x);
            max.y = max.y.max(point.y);
        }
        Some(Bounds::from_corners(min, max))
    }

    pub(crate) fn hit_position(&self, position: Point<Pixels>) -> Option<Point<Pixels>> {
        let Some(node) = &self.0 else {
            return Some(position);
        };
        let position = node.parent.hit_position(position)?;
        if !node.bounds.contains(&position) || !node.clip.contains(&position) {
            return None;
        }
        node.transform
            .hit_position(position, node.bounds, node.scale_factor)
    }
}
