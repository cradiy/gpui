use crate::{Bounds, Hitbox, HitboxId, Pixels, Point, PointerMapping, Window};
use collections::FxHashMap;
use std::{ops::Range, rc::Rc};

/// A stable handle to an element's geometry in a window.
///
/// Register source bounds with [`Window::track_element_bounds`] during prepaint.
/// Cached views retain registrations and update their mappings when transformed.
/// Queries during drawing use the frame being built; outside drawing they use the
/// completed frame. An unregistered or removed element returns `None`.
/// Use a separate handle for each element occurrence.
#[derive(Clone, Default)]
pub struct ElementBounds(Rc<()>);

impl ElementBounds {
    fn id(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }

    fn geometry<'a>(&self, window: &'a Window) -> Option<&'a Registration> {
        let frame = if window.invalidator.not_drawing() {
            &window.rendered_frame
        } else {
            &window.next_frame
        };
        let index = frame.tracked_bounds.indices.get(&self.id())?;
        frame.tracked_bounds.entries.get(*index)
    }

    /// Returns the axis-aligned bounds in window pixels. For inverse-only pointer
    /// mappings without a forward transform, returns the original source bounds.
    pub fn bounds(&self, window: &Window) -> Option<Bounds<Pixels>> {
        self.geometry(window).map(|entry| {
            entry
                .mapping
                .bounds_to_display(entry.bounds)
                .unwrap_or(entry.bounds)
        })
    }

    /// Returns the visible portion's axis-aligned window bounds, or `None` when
    /// fully clipped. Affine scopes preserve polygon clipping through rotation.
    /// Scopes without a forward map use source geometry. Occlusion by other
    /// elements is not considered.
    pub fn visible_bounds(&self, window: &Window) -> Option<Bounds<Pixels>> {
        let entry = self.geometry(window)?;
        entry.mapping.visible_bounds_to_display(
            entry.bounds.intersect(&entry.clip),
            Bounds::new(Point::default(), window.viewport_size()),
        )
    }

    /// Tests a window-space point against the source rectangle through the inverse
    /// mapping, respecting clipping and the viewport, but not other elements' occlusion.
    pub fn contains(&self, position: Point<Pixels>, window: &Window) -> bool {
        self.hit_position(position, window).is_some()
    }

    pub(crate) fn hit_position(
        &self,
        position: Point<Pixels>,
        window: &Window,
    ) -> Option<Point<Pixels>> {
        if !Bounds::new(Point::default(), window.viewport_size()).contains(&position) {
            return None;
        }
        let entry = self.geometry(window)?;
        let position = entry.mapping.hit_position(position)?;
        (entry.bounds.contains(&position) && entry.clip.contains(&position)).then_some(position)
    }

    pub(crate) fn unoccluded_hit_position(
        &self,
        position: Point<Pixels>,
        window: &Window,
    ) -> Option<Point<Pixels>> {
        let source_position = self.hit_position(position, window)?;
        let hitbox = self.geometry(window)?.hitbox?;
        let frame = if window.invalidator.not_drawing() {
            &window.rendered_frame
        } else {
            &window.next_frame
        };
        let hits = frame.hit_test(position);
        hits.ids[..hits.hover_hitbox_count]
            .contains(&hitbox)
            .then_some(source_position)
    }
}

#[derive(Clone)]
struct Registration {
    handle: ElementBounds,
    bounds: Bounds<Pixels>,
    clip: Bounds<Pixels>,
    mapping: PointerMapping,
    hitbox: Option<HitboxId>,
}

#[derive(Default)]
pub(super) struct TrackedBounds {
    entries: Vec<Registration>,
    indices: FxHashMap<usize, usize>,
}

impl TrackedBounds {
    fn push(&mut self, entry: Registration) {
        self.indices.insert(entry.handle.id(), self.entries.len());
        self.entries.push(entry);
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.indices.clear();
    }

    pub(super) fn truncate(&mut self, length: usize) {
        self.entries.truncate(length);
        self.indices.clear();
        self.indices.extend(
            self.entries
                .iter()
                .enumerate()
                .map(|(i, entry)| (entry.handle.id(), i)),
        );
    }

    pub(super) fn reuse(&mut self, previous: &Self, range: Range<usize>) {
        for entry in &previous.entries[range] {
            self.push(entry.clone());
        }
    }

    pub(super) fn can_remap(&self, range: Range<usize>, mapping: &PointerMapping) -> bool {
        self.entries[range]
            .iter()
            .all(|entry| entry.mapping == *mapping)
    }

    pub(super) fn remap(&mut self, range: Range<usize>, mapping: &PointerMapping) {
        for entry in &mut self.entries[range] {
            entry.mapping = mapping.clone();
        }
    }
}

impl Window {
    /// Registers source-space element bounds for this frame during prepaint.
    /// Read the handle after its element has prepainted, for example when positioning
    /// a deferred overlay. Cache replay keeps registrations current without invoking
    /// the element's prepaint callback again.
    pub fn track_element_bounds(&mut self, handle: &ElementBounds, bounds: Bounds<Pixels>) {
        self.invalidator.debug_assert_prepaint();
        self.next_frame.tracked_bounds.push(Registration {
            handle: handle.clone(),
            bounds,
            clip: self.content_mask().bounds,
            mapping: self.pointer_mapping.clone(),
            hitbox: None,
        });
    }

    pub(crate) fn track_element_hitbox(&mut self, handle: &ElementBounds, hitbox: &Hitbox) {
        self.invalidator.debug_assert_prepaint();
        self.next_frame.tracked_bounds.push(Registration {
            handle: handle.clone(),
            bounds: hitbox.bounds,
            clip: hitbox.content_mask.bounds,
            mapping: hitbox.pointer_mapping.clone(),
            hitbox: Some(hitbox.id),
        });
    }
}
