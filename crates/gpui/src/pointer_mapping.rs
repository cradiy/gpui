use crate::{Bounds, Pixels, Point};
use std::{fmt, rc::Rc};

/// Maps a displayed point back to source coordinates within a paint region.
#[derive(Clone)]
pub struct PointerTransform(TransformKind);

type MapPosition = dyn Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Point<Pixels>;
type HitPosition = dyn Fn(Point<Pixels>, Bounds<Pixels>, f32) -> Option<Point<Pixels>>;

#[derive(Clone)]
enum TransformKind {
    Noninteractive,
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
        Self::new(|position, _, _| position)
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
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
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
