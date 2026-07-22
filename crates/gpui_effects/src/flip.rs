use crate::four_image_effect;
use gpui::prelude::*;
use gpui::{
    Bounds, Context, EffectShader, EffectUniforms, EventEmitter, ImageSource, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, Render, Rgba,
    Window, canvas, div, img, px,
};
use std::{cell::Cell, ops::Range, rc::Rc, time::Instant};

/// Uniform slot containing `[progress, pointer_y, edge, curl_radius]`.
pub const FLIP_INTERACTION_SLOT: usize = 0;

/// Uniform slot containing `[shadow, highlight, back_brightness, reserved]`.
pub const FLIP_APPEARANCE_SLOT: usize = 1;

/// Uniform slot containing `[object_fit, layout, reserved, reserved]`.
pub const FLIP_LAYOUT_SLOT: usize = 2;

/// Uniform slot containing the crop region of the four bound images.
pub const FLIP_REGIONS_SLOT: usize = 3;

/// Uniform slot containing the RGBA color behind contained images.
pub const FLIP_BACKGROUND_SLOT: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FlipEdge {
    Left,
    Right,
}

/// Direction of a completed page turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipDirection {
    Backward,
    Forward,
}

/// Order in which logical slots are read.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FlipReadingDirection {
    /// Earlier slots are displayed on the left and forward turns start at the right edge.
    #[default]
    LeftToRight,
    /// Earlier slots are displayed on the right and forward turns start at the left edge.
    RightToLeft,
}

/// Number of logical slots visible at rest.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FlipLayout {
    /// Display and advance one slot at a time.
    Single,
    /// Display and advance two slots at a time.
    #[default]
    Spread,
}

impl FlipLayout {
    fn visible_count(self) -> usize {
        match self {
            Self::Single => 1,
            Self::Spread => 2,
        }
    }
}

/// Image fitting policy applied inside each visible page.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FlipObjectFit {
    Fill,
    #[default]
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl From<ObjectFit> for FlipObjectFit {
    fn from(value: ObjectFit) -> Self {
        match value {
            ObjectFit::Fill => Self::Fill,
            ObjectFit::Contain => Self::Contain,
            ObjectFit::Cover => Self::Cover,
            ObjectFit::ScaleDown => Self::ScaleDown,
            ObjectFit::None => Self::None,
        }
    }
}

impl FlipObjectFit {
    fn shader_value(self) -> f32 {
        match self {
            Self::Fill => 0.0,
            Self::Contain => 1.0,
            Self::Cover => 2.0,
            Self::ScaleDown => 3.0,
            Self::None => 4.0,
        }
    }
}

/// A source crop assigned to one logical slot.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FlipImageRegion {
    #[default]
    Full,
    LeftHalf,
    RightHalf,
    /// First half in logical reading order.
    ReadingStartHalf,
    /// Second half in logical reading order.
    ReadingEndHalf,
}

impl FlipImageRegion {
    fn shader_value(self, direction: FlipReadingDirection) -> f32 {
        let physical = match (self, direction) {
            (Self::ReadingStartHalf, FlipReadingDirection::LeftToRight)
            | (Self::ReadingEndHalf, FlipReadingDirection::RightToLeft) => Self::LeftHalf,
            (Self::ReadingStartHalf, FlipReadingDirection::RightToLeft)
            | (Self::ReadingEndHalf, FlipReadingDirection::LeftToRight) => Self::RightHalf,
            _ => self,
        };
        match physical {
            Self::Full => 0.0,
            Self::LeftHalf => 1.0,
            Self::RightHalf => 2.0,
            Self::ReadingStartHalf | Self::ReadingEndHalf => unreachable!(),
        }
    }
}

/// Result of requesting an animated turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipRequestResult {
    Started,
    Queued,
    Busy,
    Unavailable,
}

/// Result of changing the current logical position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipJumpResult {
    Applied,
    Busy,
    OutOfRange,
    Misaligned,
}

/// Result of changing sequence structure or content.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipUpdateResult {
    Applied,
    Busy,
    Unsupported,
    InvalidCount,
    OutOfRange,
}

/// Why the visible logical position changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipPositionReason {
    Jumped,
    Flipped(FlipDirection),
    LayoutChanged,
    SequenceResized,
}

/// Why a page sequence asked its owner to preload nearby images.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipPreloadReason {
    Initial,
    Jumped,
    Triggered(FlipDirection),
    Completed(FlipDirection),
}

/// Events emitted by [`Flip`] when it is backed by a slot sequence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlipEvent {
    /// The owner should make the specified half-open slot range available.
    PreloadRequested {
        reason: FlipPreloadReason,
        /// First logical slot of the destination view.
        anchor: usize,
        /// Number of slots requested before the visible spread.
        before: usize,
        /// Number of slots requested after the visible spread.
        after: usize,
        /// First slot index in the requested range.
        start: usize,
        /// Exclusive end slot index in the requested range.
        end: usize,
    },
    /// The visible spread changed after a flip completed.
    Flipped {
        direction: FlipDirection,
        /// First logical slot in reading order in the new view.
        position: usize,
    },
    /// The visible logical range changed, including non-animated jumps.
    PositionChanged {
        reason: FlipPositionReason,
        position: usize,
        visible_start: usize,
        visible_end: usize,
    },
    /// A provider explicitly failed to resolve a slot and the failure placeholder is used.
    SlotFailed { index: usize },
    /// A slot has completed decode and GPU prewarming.
    SlotReady { index: usize, failed: bool },
}

/// A physical slot returned by a lazy flip provider.
///
/// Higher-level readers can map logical pages to slots to insert leading or
/// trailing blanks without teaching the flip primitive about book semantics.
#[derive(Clone)]
pub enum FlipSlot {
    /// A renderable source is available for this slot.
    Source(ImageSource),
    /// A cropped part of a source is available for this slot.
    Region {
        source: ImageSource,
        region: FlipImageRegion,
    },
    /// This slot intentionally displays the provider's reusable blank source.
    Blank,
    /// The source is not available yet. A turn that requires it will wait.
    Pending,
    /// Loading failed. The reusable failure source is displayed instead.
    Failed,
}

/// One logical image item before it is mapped to physical flip slots.
///
/// A double-page item stays whole in [`FlipLayout::Single`] and expands to an
/// aligned pair of shared-texture crops in [`FlipLayout::Spread`].
#[derive(Clone)]
pub enum FlipEntry {
    Page(FlipSlot),
    DoublePage(FlipSlot),
}

impl FlipEntry {
    pub fn page(source: impl Into<ImageSource>) -> Self {
        Self::Page(FlipSlot::source(source))
    }

    pub fn double_page(source: impl Into<ImageSource>) -> Self {
        Self::DoublePage(FlipSlot::source(source))
    }

    pub fn blank() -> Self {
        Self::Page(FlipSlot::Blank)
    }

    pub fn pending() -> Self {
        Self::Page(FlipSlot::Pending)
    }

    pub fn failed() -> Self {
        Self::Page(FlipSlot::Failed)
    }
}

impl FlipSlot {
    /// Creates an available slot from any GPUI image source.
    pub fn source(source: impl Into<ImageSource>) -> Self {
        Self::Source(source.into())
    }

    /// Assigns a source crop to this slot.
    pub fn region(source: impl Into<ImageSource>, region: FlipImageRegion) -> Self {
        Self::Region {
            source: source.into(),
            region,
        }
    }

    /// Splits one horizontal image across two physical pages without duplicating its texture.
    pub fn spread(source: impl Into<ImageSource>) -> [Self; 2] {
        let source = source.into();
        [
            Self::region(source.clone(), FlipImageRegion::ReadingStartHalf),
            Self::region(source, FlipImageRegion::ReadingEndHalf),
        ]
    }
}

#[derive(Clone)]
struct SlotTexture {
    source: ImageSource,
    region: FlipImageRegion,
}

impl SlotTexture {
    fn full(source: impl Into<ImageSource>) -> Self {
        Self {
            source: source.into(),
            region: FlipImageRegion::Full,
        }
    }
}

/// Geometry preset used by [`Flip`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum FlipStyle {
    /// A mostly planar sheet with a crisp book-like turn.
    Rigid,
    /// A curved paper surface with balanced stiffness and deformation.
    #[default]
    Natural,
    /// A thinner sheet that bends inward toward the stationary surface.
    Soft,
}

/// An interactive image sequence with single-page and two-page flip layouts.
///
/// Construct this inside a GPUI context with
/// `cx.new(|_| Flip::new(previous, front, back, next))`, then add the
/// entity as a child. Pressing and dragging either outer edge starts the turn.
pub struct Flip {
    previous: SlotTexture,
    front: SlotTexture,
    back: SlotTexture,
    next: SlotTexture,
    slots: Option<Vec<SlotTexture>>,
    entries: Option<Vec<FlipEntry>>,
    slot_entries: Option<Vec<usize>>,
    slot_provider: Option<Rc<dyn Fn(usize) -> FlipSlot>>,
    blank_source: Option<ImageSource>,
    failed_source: Option<ImageSource>,
    slot_resolved: Vec<bool>,
    slot_failed: Vec<bool>,
    position: usize,
    slot_loaded: Vec<bool>,
    slot_warming: Vec<bool>,
    slot_ready: Vec<bool>,
    preload_before: usize,
    preload_after: usize,
    preload_announced: bool,
    anticipated_position: Option<usize>,
    pending_flip: Option<FlipDirection>,
    resting_sources_dirty: bool,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    active_edge: FlipEdge,
    layout: FlipLayout,
    reading_direction: FlipReadingDirection,
    object_fit: FlipObjectFit,
    background: Rgba,
    style: FlipStyle,
    shader_override: Option<EffectShader>,
    uniforms: EffectUniforms,
    progress: f32,
    pointer_y: f32,
    velocity: f32,
    target: Option<f32>,
    dragging: bool,
    trigger_width: f32,
    completion_threshold: f32,
    curl_radius: f32,
    last_frame: Instant,
}

impl Flip {
    /// Creates a spread from the visible left page, turning page front and
    /// back, and the page exposed underneath it.
    pub fn new(
        previous: impl Into<ImageSource>,
        front: impl Into<ImageSource>,
        back: impl Into<ImageSource>,
        next: impl Into<ImageSource>,
    ) -> Self {
        let previous = SlotTexture::full(previous);
        let front = SlotTexture::full(front);
        let back = SlotTexture::full(back);
        let next = SlotTexture::full(next);
        Self {
            previous,
            front,
            back,
            next,
            slots: None,
            entries: None,
            slot_entries: None,
            slot_provider: None,
            blank_source: None,
            failed_source: None,
            slot_resolved: Vec::new(),
            slot_failed: Vec::new(),
            position: 0,
            slot_loaded: Vec::new(),
            slot_warming: Vec::new(),
            slot_ready: Vec::new(),
            preload_before: 4,
            preload_after: 4,
            preload_announced: true,
            anticipated_position: None,
            pending_flip: None,
            resting_sources_dirty: false,
            bounds: Rc::new(Cell::new(Bounds::default())),
            active_edge: FlipEdge::Right,
            layout: FlipLayout::Spread,
            reading_direction: FlipReadingDirection::LeftToRight,
            object_fit: FlipObjectFit::Contain,
            background: Rgba::default(),
            style: FlipStyle::Natural,
            shader_override: None,
            uniforms: EffectUniforms::new().with_slot(FLIP_APPEARANCE_SLOT, [0.56, 0.26, 0.9, 0.0]),
            progress: 0.0,
            pointer_y: 0.5,
            velocity: 0.0,
            target: None,
            dragging: false,
            trigger_width: 42.0,
            completion_threshold: 0.42,
            curl_radius: 0.13,
            last_frame: Instant::now(),
        }
    }

    /// Creates a two-page book from an ordered page sequence.
    ///
    /// The sequence must contain an even number of slots. The component
    /// selects the four textures needed by each turn and lazily preloads a
    /// configurable window around the visible spread.
    pub fn from_slots<I, P>(slots: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<ImageSource>,
    {
        let slots = slots
            .into_iter()
            .map(|source| SlotTexture::full(source.into()))
            .collect::<Vec<_>>();
        assert!(slots.len() >= 2, "flip requires at least two slots");
        assert!(
            slots.len().is_multiple_of(2),
            "flip requires an even number of slots"
        );

        let current_left = slots[0].source.clone();
        let current_right = slots[1].source.clone();
        let mut flip = Self::new(
            current_left.clone(),
            current_right.clone(),
            current_left,
            current_right,
        );
        flip.slot_loaded = vec![false; slots.len()];
        flip.slot_warming = vec![false; slots.len()];
        flip.slot_ready = vec![false; slots.len()];
        flip.slot_failed = vec![false; slots.len()];
        flip.slot_resolved = vec![true; slots.len()];
        flip.slots = Some(slots);
        flip.preload_announced = false;
        flip
    }

    /// Creates a lazily resolved sequence of physical page slots.
    ///
    /// `slot_count` must be even and already include any leading or trailing
    /// blank slots required by the higher-level book layout. The provider is
    /// polled only for the active preload window. It may return [`Pending`](FlipSlot::Pending)
    /// and later ask the component to poll again through [`Self::refresh_slots`].
    pub fn from_provider<F>(
        slot_count: usize,
        blank_source: impl Into<ImageSource>,
        provider: F,
    ) -> Self
    where
        F: Fn(usize) -> FlipSlot + 'static,
    {
        let blank_source = blank_source.into();
        Self::from_provider_with_placeholders(
            slot_count,
            blank_source.clone(),
            blank_source,
            provider,
        )
    }

    /// Creates a lazy sequence with distinct blank and loading-failure placeholders.
    pub fn from_provider_with_placeholders<F>(
        slot_count: usize,
        blank_source: impl Into<ImageSource>,
        failed_source: impl Into<ImageSource>,
        provider: F,
    ) -> Self
    where
        F: Fn(usize) -> FlipSlot + 'static,
    {
        Self::from_provider_layout(
            slot_count,
            blank_source,
            failed_source,
            FlipLayout::Spread,
            provider,
        )
    }

    /// Creates a lazy sequence with an explicit initial layout.
    ///
    /// Single-page sequences may contain any positive number of slots. Spread
    /// sequences require an even count of at least two.
    pub fn from_provider_layout<F>(
        slot_count: usize,
        blank_source: impl Into<ImageSource>,
        failed_source: impl Into<ImageSource>,
        layout: FlipLayout,
        provider: F,
    ) -> Self
    where
        F: Fn(usize) -> FlipSlot + 'static,
    {
        assert!(
            match layout {
                FlipLayout::Single => slot_count >= 1,
                FlipLayout::Spread => slot_count >= 2 && slot_count.is_multiple_of(2),
            },
            "slot count is incompatible with flip layout"
        );

        let blank_source = blank_source.into();
        let failed_source = failed_source.into();
        let mut flip = Self::new(
            blank_source.clone(),
            blank_source.clone(),
            blank_source.clone(),
            blank_source.clone(),
        );
        flip.slots = Some(vec![SlotTexture::full(blank_source.clone()); slot_count]);
        flip.slot_provider = Some(Rc::new(provider));
        flip.blank_source = Some(blank_source);
        flip.failed_source = Some(failed_source);
        flip.slot_resolved = vec![false; slot_count];
        flip.slot_failed = vec![false; slot_count];
        flip.slot_loaded = vec![false; slot_count];
        flip.slot_warming = vec![false; slot_count];
        flip.slot_ready = vec![false; slot_count];
        flip.layout = layout;
        flip.preload_announced = false;
        flip.resting_sources_dirty = true;
        flip
    }

    /// Creates a finite sequence that may contain blanks, failures, or shared crops.
    pub fn from_slot_sequence<I>(
        slots: I,
        blank_source: impl Into<ImageSource>,
        failed_source: impl Into<ImageSource>,
        layout: FlipLayout,
    ) -> Self
    where
        I: IntoIterator<Item = FlipSlot>,
    {
        let slots = Rc::new(slots.into_iter().collect::<Vec<_>>());
        let provider_slots = slots.clone();
        Self::from_provider_layout(
            slots.len(),
            blank_source,
            failed_source,
            layout,
            move |index| provider_slots[index].clone(),
        )
    }

    /// Creates a finite logical sequence with first-class double-page items.
    ///
    /// In spread mode double-page items are automatically aligned and padded,
    /// then split into two slots sampling the same texture. In single mode the
    /// original horizontal image remains one full slot.
    pub fn from_entries<I>(
        entries: I,
        blank_source: impl Into<ImageSource>,
        failed_source: impl Into<ImageSource>,
        layout: FlipLayout,
    ) -> Self
    where
        I: IntoIterator<Item = FlipEntry>,
    {
        let entries = entries.into_iter().collect::<Vec<_>>();
        assert!(!entries.is_empty(), "flip requires at least one entry");
        let (slots, slot_entries) = Self::expand_entries_with_map(&entries, layout);
        let mut flip = Self::from_slot_sequence(slots, blank_source, failed_source, layout);
        flip.entries = Some(entries);
        flip.slot_entries = Some(slot_entries);
        flip
    }

    /// Replaces a finite logical entry sequence and rebuilds its physical mapping.
    pub fn set_entries<I>(&mut self, entries: I, cx: &mut Context<Self>) -> FlipUpdateResult
    where
        I: IntoIterator<Item = FlipEntry>,
    {
        if self.dragging || self.target.is_some() {
            return FlipUpdateResult::Busy;
        }
        if self.blank_source.is_none() {
            return FlipUpdateResult::Unsupported;
        }
        let entries = entries.into_iter().collect::<Vec<_>>();
        if entries.is_empty() {
            return FlipUpdateResult::InvalidCount;
        }
        self.entries = Some(entries);
        self.rebuild_entry_slots(self.layout);
        self.emit_preload_request(FlipPreloadReason::Jumped, cx);
        self.preload_announced = true;
        self.emit_position_changed(FlipPositionReason::SequenceResized, cx);
        cx.notify();
        FlipUpdateResult::Applied
    }

    /// Returns the first visible slot in logical reading order.
    pub fn position(&self) -> usize {
        self.position
    }

    /// Returns the zero-based physical left slot.
    ///
    /// In single-page mode this is identical to [`Self::position`].
    pub fn left_slot(&self) -> usize {
        self.physical_indices_at(self.position).0
    }

    /// Returns the half-open logical range currently visible.
    pub fn visible_range(&self) -> Range<usize> {
        self.visible_range_at(self.position)
    }

    /// Returns the active presentation layout.
    pub fn current_layout(&self) -> FlipLayout {
        self.layout
    }

    /// Returns the active logical reading direction.
    pub fn current_reading_direction(&self) -> FlipReadingDirection {
        self.reading_direction
    }

    /// Returns the active image fitting policy.
    pub fn current_object_fit(&self) -> FlipObjectFit {
        self.object_fit
    }

    /// Returns whether a logical turn is available, independent of loading readiness.
    pub fn can_flip(&self, direction: FlipDirection) -> bool {
        self.has_flip(direction)
    }

    /// Returns whether a drag, spring animation, or queued turn is active.
    pub fn is_animating(&self) -> bool {
        self.dragging || self.target.is_some() || self.pending_flip.is_some()
    }

    /// Returns a queued direction that is waiting for source textures.
    pub fn pending_direction(&self) -> Option<FlipDirection> {
        self.pending_flip
    }

    /// Selects the initial logical position before the component is rendered.
    pub fn start_at(mut self, position: usize) -> Self {
        let stride = self.layout.visible_count();
        assert!(position % stride == 0, "initial position is misaligned");
        assert!(
            self.slots
                .as_ref()
                .is_some_and(|slots| position + stride <= slots.len()),
            "initial position is outside the slot sequence"
        );
        self.position = position;
        self.configure_resting_spread();
        self.resting_sources_dirty = self.slot_provider.is_some();
        self
    }

    /// Returns the number of physical slots in sequence/provider mode.
    pub fn slot_count(&self) -> usize {
        self.slots.as_ref().map_or(4, Vec::len)
    }

    /// Marks provider-backed slots as changed and schedules them to be polled
    /// again on the next render.
    ///
    /// This is the completion hook for asynchronous loaders started in
    /// response to [`FlipEvent::PreloadRequested`].
    pub fn refresh_slots(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let Some(slots) = self.slots.as_ref() else {
            return;
        };
        assert!(
            range.start <= range.end && range.end <= slots.len(),
            "refreshed slot range is outside the provider"
        );
        if self.slot_provider.is_none() {
            return;
        }

        let visible = self.visible_range();
        let touches_visible = range.start < visible.end && range.end > visible.start;
        for index in range {
            self.slot_resolved[index] = false;
            self.slot_failed[index] = false;
            self.slot_loaded[index] = false;
            self.slot_warming[index] = false;
            self.slot_ready[index] = false;
        }
        self.resting_sources_dirty |= touches_visible;
        cx.notify();
    }

    /// Marks one provider-backed slot as changed.
    pub fn refresh_slot(&mut self, index: usize, cx: &mut Context<Self>) {
        self.refresh_slots(index..index + 1, cx);
    }

    /// Jumps to a logical position and returns to the resting state.
    pub fn set_position(&mut self, position: usize, cx: &mut Context<Self>) -> FlipJumpResult {
        if self.dragging || self.target.is_some() {
            return FlipJumpResult::Busy;
        }
        let Some(slots) = self.slots.as_ref() else {
            return FlipJumpResult::OutOfRange;
        };
        let stride = self.layout.visible_count();
        if position % stride != 0 {
            return FlipJumpResult::Misaligned;
        }
        if position + stride > slots.len() {
            return FlipJumpResult::OutOfRange;
        }
        self.position = position;
        self.configure_resting_spread();
        self.resting_sources_dirty = self.slot_provider.is_some();
        self.reset_interaction();
        self.emit_preload_request(FlipPreloadReason::Jumped, cx);
        self.emit_position_changed(FlipPositionReason::Jumped, cx);
        cx.notify();
        FlipJumpResult::Applied
    }

    /// Jumps using a physical left-slot index.
    pub fn set_left_slot(&mut self, left_slot: usize, cx: &mut Context<Self>) -> FlipJumpResult {
        let position = match (self.layout, self.reading_direction) {
            (FlipLayout::Spread, FlipReadingDirection::RightToLeft) => {
                let Some(position) = left_slot.checked_sub(1) else {
                    return FlipJumpResult::OutOfRange;
                };
                position
            }
            _ => left_slot,
        };
        self.set_position(position, cx)
    }

    /// Changes the number of provider-backed logical slots.
    pub fn set_slot_count(
        &mut self,
        slot_count: usize,
        cx: &mut Context<Self>,
    ) -> FlipUpdateResult {
        if self.dragging || self.target.is_some() {
            return FlipUpdateResult::Busy;
        }
        if self.entries.is_some() {
            return FlipUpdateResult::Unsupported;
        }
        if self.slot_provider.is_none() || self.blank_source.is_none() {
            return FlipUpdateResult::Unsupported;
        }
        if !self.valid_slot_count(slot_count) {
            return FlipUpdateResult::InvalidCount;
        }
        let blank = self.blank_source.clone().unwrap();
        let Some(slots) = self.slots.as_mut() else {
            return FlipUpdateResult::Unsupported;
        };
        slots.resize(slot_count, SlotTexture::full(blank));
        self.slot_resolved.resize(slot_count, false);
        self.slot_failed.resize(slot_count, false);
        self.slot_loaded.resize(slot_count, false);
        self.slot_warming.resize(slot_count, false);
        self.slot_ready.resize(slot_count, false);
        let stride = self.layout.visible_count();
        if self.position + stride > slot_count {
            self.position = slot_count - stride;
        }
        self.reset_interaction();
        self.resting_sources_dirty = true;
        self.emit_preload_request(FlipPreloadReason::Jumped, cx);
        self.emit_position_changed(FlipPositionReason::SequenceResized, cx);
        cx.notify();
        FlipUpdateResult::Applied
    }

    /// Assigns or replaces one slot without recreating the component.
    pub fn set_slot(
        &mut self,
        index: usize,
        slot: FlipSlot,
        cx: &mut Context<Self>,
    ) -> FlipUpdateResult {
        if self.slots.is_none() || self.entries.is_some() {
            return FlipUpdateResult::Unsupported;
        }
        if index >= self.slot_count() {
            return FlipUpdateResult::OutOfRange;
        }
        self.apply_slot(index, slot, cx);
        let visible = self.visible_range();
        self.resting_sources_dirty |= visible.contains(&index);
        cx.notify();
        FlipUpdateResult::Applied
    }

    /// Releases provider-backed slots so the provider can recreate them later.
    /// Visible slots and textures used by an active turn are retained.
    pub fn evict_slots(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> FlipUpdateResult {
        if self.slot_provider.is_none() || self.blank_source.is_none() {
            return FlipUpdateResult::Unsupported;
        }
        if range.start > range.end || range.end > self.slot_count() {
            return FlipUpdateResult::OutOfRange;
        }
        let protected = if self.dragging || self.target.is_some() {
            self.flip_range(self.direction_for_edge(self.active_edge))
        } else if let Some(direction) = self.pending_flip {
            self.flip_range(direction)
        } else {
            self.visible_range()
        };
        let blank = self.blank_source.clone().unwrap();
        let slots = self.slots.as_mut().unwrap();
        for index in range {
            if protected.contains(&index) {
                continue;
            }
            slots[index] = SlotTexture::full(blank.clone());
            self.slot_resolved[index] = false;
            self.slot_failed[index] = false;
            self.slot_loaded[index] = false;
            self.slot_warming[index] = false;
            self.slot_ready[index] = false;
        }
        cx.notify();
        FlipUpdateResult::Applied
    }

    /// Configures how many sources before and after the visible spread
    /// should be lazily prepared.
    ///
    /// The two visible slots are always included. Adjacent slots required by a
    /// triggered turn are also requested even when either value is zero.
    pub fn preload_slots(mut self, before: usize, after: usize) -> Self {
        self.preload_before = before;
        self.preload_after = after;
        self
    }

    /// Updates the lazy preload window at runtime.
    pub fn set_preload_slots(&mut self, before: usize, after: usize, cx: &mut Context<Self>) {
        self.preload_before = before;
        self.preload_after = after;
        self.emit_preload_request(FlipPreloadReason::Jumped, cx);
        cx.notify();
    }

    /// Selects single-page or two-page spread presentation.
    pub fn layout(mut self, layout: FlipLayout) -> Self {
        if self.entries.is_some() {
            self.rebuild_entry_slots(layout);
            return self;
        }
        assert!(
            self.slots.as_ref().is_none_or(|slots| match layout {
                FlipLayout::Single => !slots.is_empty(),
                FlipLayout::Spread => slots.len() >= 2 && slots.len().is_multiple_of(2),
            }),
            "slot count is incompatible with flip layout"
        );
        self.layout = layout;
        self.configure_resting_spread();
        self
    }

    /// Changes presentation without replacing the slot provider.
    pub fn set_layout(&mut self, layout: FlipLayout, cx: &mut Context<Self>) -> FlipUpdateResult {
        if self.dragging || self.target.is_some() {
            return FlipUpdateResult::Busy;
        }
        if self.entries.is_some() {
            self.rebuild_entry_slots(layout);
            self.emit_preload_request(FlipPreloadReason::Jumped, cx);
            self.preload_announced = true;
            self.emit_position_changed(FlipPositionReason::LayoutChanged, cx);
            cx.notify();
            return FlipUpdateResult::Applied;
        }
        let count = self.slot_count();
        let valid = match layout {
            FlipLayout::Single => count >= 1,
            FlipLayout::Spread => count >= 2 && count.is_multiple_of(2),
        };
        if !valid {
            return FlipUpdateResult::InvalidCount;
        }
        self.layout = layout;
        let stride = layout.visible_count();
        self.position -= self.position % stride;
        if self.position + stride > count {
            self.position = count - stride;
        }
        self.reset_interaction();
        self.configure_resting_spread();
        self.resting_sources_dirty = self.slot_provider.is_some();
        self.emit_preload_request(FlipPreloadReason::Jumped, cx);
        self.emit_position_changed(FlipPositionReason::LayoutChanged, cx);
        cx.notify();
        FlipUpdateResult::Applied
    }

    /// Selects the logical reading direction while keeping provider slots in reading order.
    pub fn reading_direction(mut self, direction: FlipReadingDirection) -> Self {
        self.reading_direction = direction;
        self.configure_resting_spread();
        self
    }

    pub fn set_reading_direction(
        &mut self,
        direction: FlipReadingDirection,
        cx: &mut Context<Self>,
    ) -> FlipUpdateResult {
        if self.dragging || self.target.is_some() {
            return FlipUpdateResult::Busy;
        }
        self.reading_direction = direction;
        self.reset_interaction();
        self.configure_resting_spread();
        cx.notify();
        FlipUpdateResult::Applied
    }

    /// Selects how each slot image is fitted into its page.
    pub fn object_fit(mut self, object_fit: impl Into<FlipObjectFit>) -> Self {
        self.object_fit = object_fit.into();
        self
    }

    pub fn set_object_fit(&mut self, object_fit: impl Into<FlipObjectFit>, cx: &mut Context<Self>) {
        self.object_fit = object_fit.into();
        cx.notify();
    }

    /// Sets the color visible around images fitted with `Contain`, `ScaleDown`, or `None`.
    pub fn page_background(mut self, background: Rgba) -> Self {
        self.background = background;
        self
    }

    pub fn set_page_background(&mut self, background: Rgba, cx: &mut Context<Self>) {
        self.background = background;
        cx.notify();
    }

    /// Sets the activation distance from either outer page edge.
    pub fn trigger_width(mut self, width: Pixels) -> Self {
        self.trigger_width = f32::from(width).max(1.0);
        self
    }

    /// Sets the drag progress after which release completes the turn.
    pub fn completion_threshold(mut self, threshold: f32) -> Self {
        self.completion_threshold = threshold.clamp(0.05, 0.95);
        self
    }

    /// Sets the normalized radius of the simulated curled sheet.
    pub fn curl_radius(mut self, radius: f32) -> Self {
        self.curl_radius = radius.clamp(0.04, 0.28);
        self
    }

    /// Selects the page geometry preset.
    pub fn style(mut self, style: FlipStyle) -> Self {
        self.style = style;
        self.shader_override = None;
        self
    }

    /// Overrides the preset with a custom four-image effect shader.
    ///
    /// Custom WGSL can be created with [`EffectShader::wgsl_four_images`].
    /// The image helpers map to front, back, previous, and next respectively;
    /// the two standard uniform slots are documented by
    /// [`FLIP_INTERACTION_SLOT`] and [`FLIP_APPEARANCE_SLOT`].
    pub fn shader(mut self, shader: EffectShader) -> Self {
        assert_eq!(
            shader.image_count(),
            4,
            "flip shaders require four image textures"
        );
        self.shader_override = Some(shader);
        self
    }

    /// Sets a custom shader uniform slot.
    ///
    /// Slots zero through four are owned by `Flip`. Custom shaders may use
    /// slots five through seven.
    pub fn uniform(mut self, index: usize, value: [f32; 4]) -> Self {
        assert!(
            index >= 5,
            "flip uniform slots zero through four are reserved"
        );
        self.uniforms.set_slot(index, value);
        self
    }

    /// Changes the page geometry preset and returns to the resting spread.
    pub fn set_style(&mut self, style: FlipStyle, cx: &mut Context<Self>) {
        self.style = style;
        self.shader_override = None;
        self.reset_interaction();
        cx.notify();
    }

    /// Replaces the current preset with a custom four-image shader.
    pub fn set_shader(&mut self, shader: EffectShader, cx: &mut Context<Self>) {
        assert_eq!(
            shader.image_count(),
            4,
            "flip shaders require four image textures"
        );
        self.shader_override = Some(shader);
        self.reset_interaction();
        cx.notify();
    }

    /// Updates a custom shader uniform slot at runtime.
    pub fn set_uniform(&mut self, index: usize, value: [f32; 4], cx: &mut Context<Self>) {
        assert!(
            index >= 5,
            "flip uniform slots zero through four are reserved"
        );
        self.uniforms.set_slot(index, value);
        cx.notify();
    }

    fn reset_interaction(&mut self) {
        self.progress = 0.0;
        self.velocity = 0.0;
        self.target = None;
        self.anticipated_position = None;
        self.pending_flip = None;
        self.dragging = false;
    }

    /// Animates the active sheet back to its resting position.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.target = Some(0.0);
        self.dragging = false;
        self.last_frame = Instant::now();
        cx.notify();
    }

    /// Advances one layout-sized view programmatically using the current geometry preset.
    ///
    pub fn flip(&mut self, direction: FlipDirection, cx: &mut Context<Self>) -> FlipRequestResult {
        if self.dragging || self.target.is_some() || self.pending_flip.is_some() {
            return FlipRequestResult::Busy;
        }
        let edge = self.edge_for_direction(direction);
        if !self.has_flip(direction) {
            return FlipRequestResult::Unavailable;
        }

        self.anticipated_position = self.destination_position(direction);
        self.emit_preload_request(FlipPreloadReason::Triggered(direction), cx);
        if !self.flip_ready(direction) {
            self.pending_flip = Some(direction);
            cx.notify();
            return FlipRequestResult::Queued;
        }

        self.start_programmatic_turn(direction, edge);
        cx.notify();
        FlipRequestResult::Started
    }

    fn start_programmatic_turn(&mut self, direction: FlipDirection, edge: FlipEdge) {
        self.active_edge = edge;
        self.configure_sequence_edge(direction, edge);
        self.progress = 0.015;
        self.pointer_y = 0.5;
        self.velocity = 0.0;
        self.target = Some(1.0);
        self.last_frame = Instant::now();
    }

    /// Turns to the previous logical view when one is available.
    pub fn flip_backward(&mut self, cx: &mut Context<Self>) -> FlipRequestResult {
        self.flip(FlipDirection::Backward, cx)
    }

    /// Turns to the next logical view when one is available.
    pub fn flip_forward(&mut self, cx: &mut Context<Self>) -> FlipRequestResult {
        self.flip(FlipDirection::Forward, cx)
    }

    fn edge_at(&self, x: f32) -> Option<FlipEdge> {
        let bounds = self.bounds.get();
        let left = f32::from(bounds.origin.x);
        let right = f32::from(bounds.origin.x) + f32::from(bounds.size.width);
        if (x - left).abs() <= self.trigger_width
            && self.has_flip(self.direction_for_edge(FlipEdge::Left))
        {
            Some(FlipEdge::Left)
        } else if (right - x).abs() <= self.trigger_width
            && self.has_flip(self.direction_for_edge(FlipEdge::Right))
        {
            Some(FlipEdge::Right)
        } else {
            None
        }
    }

    fn has_flip(&self, direction: FlipDirection) -> bool {
        if self.slots.is_none() {
            return true;
        }
        self.destination_position(direction).is_some()
    }

    fn flip_ready(&self, direction: FlipDirection) -> bool {
        if self.slots.is_none() {
            return true;
        }
        let mut range = self.flip_range(direction);
        range.all(|index| self.slot_ready.get(index).copied().unwrap_or(false))
    }

    fn flip_range(&self, direction: FlipDirection) -> Range<usize> {
        let destination = self
            .destination_position(direction)
            .unwrap_or(self.position);
        let start = self.position.min(destination);
        let end =
            (self.position.max(destination) + self.layout.visible_count()).min(self.slot_count());
        start..end
    }

    fn destination_position(&self, direction: FlipDirection) -> Option<usize> {
        let stride = self.layout.visible_count();
        match direction {
            FlipDirection::Backward => self.position.checked_sub(stride),
            FlipDirection::Forward => {
                let destination = self.position + stride;
                (destination + stride <= self.slots.as_ref()?.len()).then_some(destination)
            }
        }
    }

    fn edge_for_direction(&self, direction: FlipDirection) -> FlipEdge {
        match (self.reading_direction, direction) {
            (FlipReadingDirection::LeftToRight, FlipDirection::Backward)
            | (FlipReadingDirection::RightToLeft, FlipDirection::Forward) => FlipEdge::Left,
            (FlipReadingDirection::LeftToRight, FlipDirection::Forward)
            | (FlipReadingDirection::RightToLeft, FlipDirection::Backward) => FlipEdge::Right,
        }
    }

    fn direction_for_edge(&self, edge: FlipEdge) -> FlipDirection {
        match (self.reading_direction, edge) {
            (FlipReadingDirection::LeftToRight, FlipEdge::Left)
            | (FlipReadingDirection::RightToLeft, FlipEdge::Right) => FlipDirection::Backward,
            (FlipReadingDirection::LeftToRight, FlipEdge::Right)
            | (FlipReadingDirection::RightToLeft, FlipEdge::Left) => FlipDirection::Forward,
        }
    }

    fn visible_range_at(&self, position: usize) -> Range<usize> {
        position..(position + self.layout.visible_count()).min(self.slot_count())
    }

    fn physical_indices_at(&self, position: usize) -> (usize, usize) {
        match (self.layout, self.reading_direction) {
            (FlipLayout::Single, _) => (position, position),
            (FlipLayout::Spread, FlipReadingDirection::LeftToRight) => (position, position + 1),
            (FlipLayout::Spread, FlipReadingDirection::RightToLeft) => (position + 1, position),
        }
    }

    fn valid_slot_count(&self, slot_count: usize) -> bool {
        match self.layout {
            FlipLayout::Single => slot_count >= 1,
            FlipLayout::Spread => slot_count >= 2 && slot_count.is_multiple_of(2),
        }
    }

    fn expand_entries_with_map(
        entries: &[FlipEntry],
        layout: FlipLayout,
    ) -> (Vec<FlipSlot>, Vec<usize>) {
        let mut slots = Vec::new();
        let mut slot_entries = Vec::new();
        for (entry_index, entry) in entries.iter().enumerate() {
            match (layout, entry) {
                (_, FlipEntry::Page(slot)) | (FlipLayout::Single, FlipEntry::DoublePage(slot)) => {
                    slots.push(slot.clone());
                    slot_entries.push(entry_index);
                }
                (FlipLayout::Spread, FlipEntry::DoublePage(slot)) => {
                    if !slots.len().is_multiple_of(2) {
                        slots.push(FlipSlot::Blank);
                        slot_entries.push(entry_index.saturating_sub(1));
                    }
                    match slot {
                        FlipSlot::Source(source) | FlipSlot::Region { source, .. } => {
                            slots.extend(FlipSlot::spread(source.clone()));
                        }
                        FlipSlot::Blank => {
                            slots.extend([FlipSlot::Blank, FlipSlot::Blank]);
                        }
                        FlipSlot::Pending => {
                            slots.extend([FlipSlot::Pending, FlipSlot::Pending]);
                        }
                        FlipSlot::Failed => {
                            slots.extend([FlipSlot::Failed, FlipSlot::Failed]);
                        }
                    }
                    slot_entries.extend([entry_index, entry_index]);
                }
            }
        }
        if layout == FlipLayout::Spread && !slots.len().is_multiple_of(2) {
            slots.push(FlipSlot::Blank);
            slot_entries.push(entries.len().saturating_sub(1));
        }
        (slots, slot_entries)
    }

    fn rebuild_entry_slots(&mut self, layout: FlipLayout) {
        let entries = self.entries.as_ref().expect("entry sequence is missing");
        let current_entry = self
            .slot_entries
            .as_ref()
            .and_then(|mapping| mapping.get(self.position))
            .copied()
            .unwrap_or(0)
            .min(entries.len().saturating_sub(1));
        let (expanded, slot_entries) = Self::expand_entries_with_map(entries, layout);
        let position = slot_entries
            .iter()
            .position(|entry| *entry == current_entry)
            .unwrap_or(0);
        let expanded = Rc::new(expanded);
        let provider_slots = expanded.clone();
        let count = expanded.len();
        let blank = self.blank_source.clone().expect("blank source is missing");
        self.slots = Some(vec![SlotTexture::full(blank); count]);
        self.slot_provider = Some(Rc::new(move |index| provider_slots[index].clone()));
        self.slot_resolved = vec![false; count];
        self.slot_failed = vec![false; count];
        self.slot_loaded = vec![false; count];
        self.slot_warming = vec![false; count];
        self.slot_ready = vec![false; count];
        self.slot_entries = Some(slot_entries);
        self.layout = layout;
        self.position = position - position % layout.visible_count();
        self.preload_announced = false;
        self.resting_sources_dirty = true;
        self.reset_interaction();
        self.configure_resting_spread();
    }

    fn preload_range_at(&self, position: usize) -> Option<std::ops::Range<usize>> {
        let slots = self.slots.as_ref()?;
        let start = position.saturating_sub(self.preload_before);
        let end = (position + self.layout.visible_count() + self.preload_after).min(slots.len());
        Some(start..end)
    }

    fn preload_range(&self) -> Option<std::ops::Range<usize>> {
        self.preload_range_at(self.position)
    }

    fn emit_preload_request(&mut self, reason: FlipPreloadReason, cx: &mut Context<Self>) {
        let anchor = match reason {
            FlipPreloadReason::Triggered(direction) => self
                .destination_position(direction)
                .unwrap_or(self.position),
            _ => self.position,
        };
        let Some(mut range) = self.preload_range_at(anchor) else {
            return;
        };
        if let FlipPreloadReason::Triggered(direction) = reason {
            let flip_range = self.flip_range(direction);
            range.start = range.start.min(flip_range.start);
            range.end = range.end.max(flip_range.end);
        }
        let before = anchor - range.start;
        let after = range.end - (anchor + self.layout.visible_count());
        cx.emit(FlipEvent::PreloadRequested {
            reason,
            anchor,
            before,
            after,
            start: range.start,
            end: range.end,
        });
    }

    fn emit_position_changed(&mut self, reason: FlipPositionReason, cx: &mut Context<Self>) {
        let visible = self.visible_range();
        cx.emit(FlipEvent::PositionChanged {
            reason,
            position: self.position,
            visible_start: visible.start,
            visible_end: visible.end,
        });
    }

    fn load_slot_range(
        &mut self,
        range: std::ops::Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.resolve_slot_range(range.clone(), cx);
        if self.slots.is_none() {
            return;
        }
        for index in range {
            if !self.slot_resolved[index] {
                continue;
            }
            let source = self.slots.as_ref().unwrap()[index].source.clone();
            match source.use_data(None, window, cx) {
                Some(Ok(_)) => self.slot_loaded[index] = true,
                Some(Err(_)) if !self.slot_failed[index] => {
                    self.apply_slot(index, FlipSlot::Failed, cx);
                }
                Some(Err(_)) | None => self.slot_loaded[index] = false,
            }
        }
    }

    fn resolve_slot_range(&mut self, range: std::ops::Range<usize>, cx: &mut Context<Self>) {
        let Some(provider) = self.slot_provider.clone() else {
            return;
        };

        for index in range {
            if self.slot_resolved[index] {
                continue;
            }
            self.apply_slot(index, provider(index), cx);
        }
    }

    fn apply_slot(&mut self, index: usize, slot: FlipSlot, cx: &mut Context<Self>) {
        let (texture, resolved, failed) = match slot {
            FlipSlot::Source(source) => (SlotTexture::full(source), true, false),
            FlipSlot::Region { source, region } => (SlotTexture { source, region }, true, false),
            FlipSlot::Blank => {
                let Some(source) = self.blank_source.clone() else {
                    return;
                };
                (SlotTexture::full(source), true, false)
            }
            FlipSlot::Pending => {
                self.slot_resolved[index] = false;
                self.slot_loaded[index] = false;
                self.slot_warming[index] = false;
                self.slot_ready[index] = false;
                return;
            }
            FlipSlot::Failed => {
                let Some(source) = self
                    .failed_source
                    .clone()
                    .or_else(|| self.blank_source.clone())
                else {
                    return;
                };
                (SlotTexture::full(source), true, true)
            }
        };
        if let Some(slots) = self.slots.as_mut() {
            slots[index] = texture;
        }
        self.slot_resolved[index] = resolved;
        self.slot_failed[index] = failed;
        self.slot_loaded[index] = false;
        self.slot_warming[index] = false;
        self.slot_ready[index] = false;
        if failed {
            cx.emit(FlipEvent::SlotFailed { index });
        }
        if self.visible_range().contains(&index) {
            self.resting_sources_dirty = true;
        }
    }

    fn configure_sequence_edge(&mut self, direction: FlipDirection, edge: FlipEdge) {
        let Some(slots) = self.slots.as_ref() else {
            return;
        };
        let Some(destination) = self.destination_position(direction) else {
            return;
        };
        let (current_left, current_right) = self.physical_indices_at(self.position);
        let (destination_left, destination_right) = self.physical_indices_at(destination);
        match edge {
            FlipEdge::Right => {
                self.front = slots[current_right].clone();
                self.back = slots[destination_left].clone();
                self.previous = slots[current_left].clone();
                self.next = slots[destination_right].clone();
            }
            FlipEdge::Left => {
                self.front = slots[current_right].clone();
                self.back = slots[destination_left].clone();
                self.previous = slots[current_left].clone();
                self.next = slots[destination_right].clone();
            }
        }
    }

    fn configure_resting_spread(&mut self) {
        let Some(slots) = self.slots.as_ref() else {
            return;
        };
        let (left, right) = self.physical_indices_at(self.position);
        let current_left = slots[left].clone();
        let current_right = slots[right].clone();

        // At progress zero only `previous` and `front` are visible. Reusing
        // those two ready images in the hidden slots keeps the four-texture
        // effect paintable while the next lazy preload window is still being
        // decoded and uploaded.
        self.previous = current_left.clone();
        self.front = current_right.clone();
        self.back = current_left;
        self.next = current_right;
    }

    fn finish_sequence_flip(&mut self, cx: &mut Context<Self>) {
        if self.slots.is_none() {
            return;
        }
        let direction = self.direction_for_edge(self.active_edge);
        self.position = self
            .destination_position(direction)
            .expect("completed flip must have a destination");
        self.configure_resting_spread();
        self.resting_sources_dirty = false;
        self.reset_interaction();
        self.emit_preload_request(FlipPreloadReason::Completed(direction), cx);
        cx.emit(FlipEvent::Flipped {
            direction,
            position: self.position,
        });
        self.emit_position_changed(FlipPositionReason::Flipped(direction), cx);
        cx.notify();
    }

    fn normalized_pointer(&self, x: f32, y: f32) -> (f32, f32) {
        let bounds = self.bounds.get();
        let width = f32::from(bounds.size.width).max(1.0);
        let height = f32::from(bounds.size.height).max(1.0);
        let left = f32::from(bounds.origin.x);
        let right = left + width;
        let progress = match self.active_edge {
            FlipEdge::Left => (x - left) / width,
            FlipEdge::Right => (right - x) / width,
        };
        (
            progress.clamp(0.0, 1.0),
            ((y - f32::from(bounds.origin.y)) / height).clamp(0.0, 1.0),
        )
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edge) = self.edge_at(f32::from(event.position.x)) else {
            return;
        };
        let direction = self.direction_for_edge(edge);
        self.anticipated_position = self.destination_position(direction);
        self.emit_preload_request(FlipPreloadReason::Triggered(direction), cx);
        self.load_slot_range(self.flip_range(direction), window, cx);
        if !self.flip_ready(direction) {
            cx.notify();
            return;
        }
        self.active_edge = edge;
        self.configure_sequence_edge(direction, edge);
        let (progress, pointer_y) =
            self.normalized_pointer(f32::from(event.position.x), f32::from(event.position.y));
        self.progress = progress.max(0.015);
        self.pointer_y = pointer_y;
        self.velocity = 0.0;
        self.target = None;
        self.dragging = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging {
            let (progress, pointer_y) =
                self.normalized_pointer(f32::from(event.position.x), f32::from(event.position.y));
            self.velocity = progress - self.progress;
            self.progress = progress;
            self.pointer_y = pointer_y;
            cx.notify();
            return;
        }
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        let complete = self.progress + self.velocity * 5.0 >= self.completion_threshold;
        self.target = Some(if complete { 1.0 } else { 0.0 });
        self.last_frame = Instant::now();
        cx.notify();
    }
}

impl Render for Flip {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Promote slots only after their invisible prewarm sprite was
        // submitted in the previous frame. CPU decoding alone is not enough:
        // the image must already have an atlas tile before a turn can use it.
        for index in 0..self.slot_warming.len() {
            if self.slot_warming[index] {
                self.slot_warming[index] = false;
                self.slot_ready[index] = true;
                cx.emit(FlipEvent::SlotReady {
                    index,
                    failed: self.slot_failed[index],
                });
            }
        }

        if !self.preload_announced {
            self.preload_announced = true;
            self.emit_preload_request(FlipPreloadReason::Initial, cx);
        }
        if let Some(mut range) = self.preload_range() {
            if let Some(anticipated_position) = self.anticipated_position
                && let Some(anticipated_range) = self.preload_range_at(anticipated_position)
            {
                range.start = range.start.min(anticipated_range.start);
                range.end = range.end.max(anticipated_range.end);
            }
            if let Some(direction) = self.pending_flip {
                let flip_range = self.flip_range(direction);
                range.start = range.start.min(flip_range.start);
                range.end = range.end.max(flip_range.end);
            }
            self.load_slot_range(range, window, cx);
        }

        let mut prewarm_images = Vec::new();
        for index in 0..self.slot_loaded.len() {
            if self.slot_loaded[index] && !self.slot_ready[index] && !self.slot_warming[index] {
                self.slot_warming[index] = true;
                if let Some(slots) = self.slots.as_ref() {
                    prewarm_images.push(slots[index].source.clone());
                }
            }
        }
        if !prewarm_images.is_empty() {
            window.request_animation_frame();
        }

        if self.resting_sources_dirty
            && !self.dragging
            && self.target.is_none()
            && self.pending_flip.is_none()
            && self
                .visible_range()
                .all(|index| self.slot_ready.get(index).copied().unwrap_or(false))
        {
            self.configure_resting_spread();
            self.resting_sources_dirty = false;
        }

        if let Some(direction) = self.pending_flip {
            let edge = self.edge_for_direction(direction);
            if self.flip_ready(direction) {
                self.pending_flip = None;
                self.start_programmatic_turn(direction, edge);
            }
        }

        if let Some(target) = self.target {
            let now = Instant::now();
            let dt = (now - self.last_frame)
                .as_secs_f32()
                .clamp(1.0 / 240.0, 1.0 / 30.0);
            self.last_frame = now;
            let (stiffness, damping) = match self.style {
                FlipStyle::Rigid => (240.0, 28.0),
                FlipStyle::Natural => (190.0, 24.0),
                FlipStyle::Soft => (125.0, 17.0),
            };
            self.velocity += (target - self.progress) * stiffness * dt;
            self.velocity *= (-damping * dt).exp();
            self.progress = (self.progress + self.velocity * dt).clamp(0.0, 1.0);
            if (self.progress - target).abs() < 0.001 && self.velocity.abs() < 0.01 {
                self.progress = target;
                self.velocity = 0.0;
                self.target = None;
                if target == 1.0 {
                    self.finish_sequence_flip(cx);
                } else {
                    self.anticipated_position = None;
                }
            } else {
                window.request_animation_frame();
            }
        }

        let bounds = self.bounds.clone();
        let edge = if self.active_edge == FlipEdge::Left {
            -1.0
        } else {
            1.0
        };
        let shader = self
            .shader_override
            .clone()
            .unwrap_or_else(|| flip_shader_for(self.style));
        let mut uniforms = self.uniforms;
        uniforms.set_slot(
            FLIP_INTERACTION_SLOT,
            [self.progress, self.pointer_y, edge, self.curl_radius],
        );
        uniforms.set_slot(
            FLIP_LAYOUT_SLOT,
            [
                self.object_fit.shader_value(),
                if self.layout == FlipLayout::Single {
                    1.0
                } else {
                    0.0
                },
                0.0,
                0.0,
            ],
        );
        uniforms.set_slot(
            FLIP_REGIONS_SLOT,
            [
                self.front.region.shader_value(self.reading_direction),
                self.back.region.shader_value(self.reading_direction),
                self.previous.region.shader_value(self.reading_direction),
                self.next.region.shader_value(self.reading_direction),
            ],
        );
        uniforms.set_slot(
            FLIP_BACKGROUND_SLOT,
            [
                self.background.r,
                self.background.g,
                self.background.b,
                self.background.a,
            ],
        );
        let effect = four_image_effect(
            self.front.source.clone(),
            self.back.source.clone(),
            self.previous.source.clone(),
            self.next.source.clone(),
            shader,
        )
        .uniforms(uniforms)
        .size_full();

        div()
            .id("gpui-flip")
            .relative()
            .size_full()
            .overflow_hidden()
            .cursor(gpui::CursorStyle::PointingHand)
            .child(effect)
            .child(
                div().absolute().inset_0().opacity(0.0).children(
                    prewarm_images
                        .into_iter()
                        .map(|source| img(source).w(px(1.)).h(px(1.))),
                ),
            )
            .child(
                canvas(
                    move |new_bounds, _, _| bounds.set(new_bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
    }
}

impl EventEmitter<FlipEvent> for Flip {}

/// Returns the portable four-image shader used by [`Flip`].
pub fn flip_shader() -> EffectShader {
    EffectShader::wgsl_four_images(flip_shader_source(include_str!("shaders/flip.wgsl")))
}

/// Returns the mostly planar page-turn shader.
pub fn rigid_flip_shader() -> EffectShader {
    EffectShader::wgsl_four_images(flip_shader_source(include_str!("shaders/flip_rigid.wgsl")))
}

/// Returns the flexible thin-paper page-turn shader.
pub fn soft_flip_shader() -> EffectShader {
    EffectShader::wgsl_four_images(flip_shader_source(include_str!("shaders/flip_soft.wgsl")))
}

fn flip_shader_source(effect: &str) -> String {
    format!("{}\n{}", include_str!("shaders/flip_sampling.wgsl"), effect)
}

/// Returns the shader associated with a flip preset.
pub fn flip_shader_for(style: FlipStyle) -> EffectShader {
    match style {
        FlipStyle::Rigid => rigid_flip_shader(),
        FlipStyle::Natural => flip_shader(),
        FlipStyle::Soft => soft_flip_shader(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spread_flip() -> Flip {
        Flip::from_provider(6, "blank", |_| FlipSlot::Blank)
    }

    #[test]
    fn reading_direction_keeps_slots_in_logical_order() {
        let mut flip = spread_flip();
        assert_eq!(flip.physical_indices_at(0), (0, 1));
        assert_eq!(
            flip.edge_for_direction(FlipDirection::Forward),
            FlipEdge::Right
        );
        assert_eq!(flip.destination_position(FlipDirection::Forward), Some(2));

        flip.reading_direction = FlipReadingDirection::RightToLeft;
        assert_eq!(flip.physical_indices_at(0), (1, 0));
        assert_eq!(
            flip.edge_for_direction(FlipDirection::Forward),
            FlipEdge::Left
        );
        assert_eq!(flip.destination_position(FlipDirection::Forward), Some(2));
    }

    #[test]
    fn single_layout_supports_odd_slot_counts() {
        let mut flip = Flip::from_provider_layout(3, "blank", "failed", FlipLayout::Single, |_| {
            FlipSlot::Blank
        });
        assert_eq!(flip.visible_range(), 0..1);
        assert_eq!(flip.destination_position(FlipDirection::Forward), Some(1));
        flip.position = 2;
        assert_eq!(flip.visible_range(), 2..3);
        assert_eq!(flip.destination_position(FlipDirection::Forward), None);
    }

    #[test]
    fn shared_spread_regions_follow_reading_direction() {
        let [first, second] = FlipSlot::spread("wide");
        let FlipSlot::Region {
            region: first_region,
            ..
        } = first
        else {
            panic!("spread start must be a cropped source");
        };
        let FlipSlot::Region {
            region: second_region,
            ..
        } = second
        else {
            panic!("spread end must be a cropped source");
        };

        assert_eq!(
            first_region.shader_value(FlipReadingDirection::LeftToRight),
            1.0
        );
        assert_eq!(
            second_region.shader_value(FlipReadingDirection::LeftToRight),
            2.0
        );
        assert_eq!(
            first_region.shader_value(FlipReadingDirection::RightToLeft),
            2.0
        );
        assert_eq!(
            second_region.shader_value(FlipReadingDirection::RightToLeft),
            1.0
        );
    }

    #[test]
    fn double_page_entry_is_one_single_page_or_one_aligned_spread() {
        let entries = [
            FlipEntry::page("portrait"),
            FlipEntry::double_page("wide"),
            FlipEntry::page("tail"),
        ];

        let single = Flip::expand_entries_with_map(&entries, FlipLayout::Single).0;
        assert_eq!(single.len(), 3);
        assert!(matches!(&single[1], FlipSlot::Source(_)));

        let spread = Flip::expand_entries_with_map(&entries, FlipLayout::Spread).0;
        assert_eq!(spread.len(), 6);
        assert!(matches!(&spread[1], FlipSlot::Blank));
        assert!(matches!(
            &spread[2],
            FlipSlot::Region {
                region: FlipImageRegion::ReadingStartHalf,
                ..
            }
        ));
        assert!(matches!(
            &spread[3],
            FlipSlot::Region {
                region: FlipImageRegion::ReadingEndHalf,
                ..
            }
        ));
        assert!(matches!(&spread[5], FlipSlot::Blank));

        let mut flip =
            Flip::from_entries(entries, "blank", "failed", FlipLayout::Spread).start_at(2);
        flip.rebuild_entry_slots(FlipLayout::Single);
        assert_eq!(flip.position(), 1);
        assert!(matches!(
            flip.slots.as_ref().unwrap()[1].region,
            FlipImageRegion::Full
        ));
    }
}
