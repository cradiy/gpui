use crate::{Bounds, Pixels, Point, PointerMapping, Window};
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

    /// Tests a window-space point against the source rectangle through the inverse
    /// mapping. This accounts for rotation, but not clipping or occluding elements.
    pub fn contains(&self, position: Point<Pixels>, window: &Window) -> bool {
        self.geometry(window).is_some_and(|entry| {
            entry
                .mapping
                .hit_position(position)
                .is_some_and(|position| entry.bounds.contains(&position))
        })
    }
}

#[derive(Clone)]
struct Registration {
    handle: ElementBounds,
    bounds: Bounds<Pixels>,
    mapping: PointerMapping,
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
            mapping: self.pointer_mapping.clone(),
        });
    }
}
