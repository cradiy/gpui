use std::{cell::Cell, rc::Rc};

use gpui::{
    AccessibleAction, App, Background, Bounds, DispatchPhase, Element, ElementId, GlobalElementId,
    Hitbox, HitboxBehavior, HitboxId, InspectorElementId, IntoElement, ListState, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Orientation, Pixels, Point, Refineable as _,
    RenderOnce, Role, ScrollHandle, Size, Style, StyleRefinement, Styled, UniformListScrollHandle,
    Window, div, fill, point, prelude::*, px, size, transparent_black,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScrollbarOrientation {
    Horizontal,
    #[default]
    Vertical,
}

impl ScrollbarOrientation {
    fn accessibility(self) -> Orientation {
        match self {
            Self::Horizontal => Orientation::Horizontal,
            Self::Vertical => Orientation::Vertical,
        }
    }

    fn coordinate(self, point: Point<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => point.x,
            Self::Vertical => point.y,
        }
    }

    fn length(self, size: Size<Pixels>) -> Pixels {
        match self {
            Self::Horizontal => size.width,
            Self::Vertical => size.height,
        }
    }
}

/// A scroll model supported by [`Scrollbar`].
#[derive(Clone, Debug)]
pub enum ScrollbarSource {
    Scroll(ScrollHandle),
    List(ListState),
    UniformList(UniformListScrollHandle),
}

impl From<ScrollHandle> for ScrollbarSource {
    fn from(value: ScrollHandle) -> Self {
        Self::Scroll(value)
    }
}

impl From<&ScrollHandle> for ScrollbarSource {
    fn from(value: &ScrollHandle) -> Self {
        Self::Scroll(value.clone())
    }
}

impl From<ListState> for ScrollbarSource {
    fn from(value: ListState) -> Self {
        Self::List(value)
    }
}

impl From<&ListState> for ScrollbarSource {
    fn from(value: &ListState) -> Self {
        Self::List(value.clone())
    }
}

impl From<UniformListScrollHandle> for ScrollbarSource {
    fn from(value: UniformListScrollHandle) -> Self {
        Self::UniformList(value)
    }
}

impl From<&UniformListScrollHandle> for ScrollbarSource {
    fn from(value: &UniformListScrollHandle) -> Self {
        Self::UniformList(value.clone())
    }
}

impl ScrollbarSource {
    fn offset(&self) -> Point<Pixels> {
        match self {
            Self::Scroll(handle) => handle.offset(),
            Self::List(state) => state.scroll_px_offset_for_scrollbar(),
            Self::UniformList(handle) => handle.0.borrow().base_handle.offset(),
        }
    }

    fn max_offset(&self) -> Point<Pixels> {
        match self {
            Self::Scroll(handle) => handle.max_offset(),
            Self::List(state) => state.max_offset_for_scrollbar(),
            Self::UniformList(handle) => handle.0.borrow().base_handle.max_offset(),
        }
    }

    fn viewport_size(&self) -> Size<Pixels> {
        match self {
            Self::Scroll(handle) => handle.bounds().size,
            Self::List(state) => state.viewport_bounds().size,
            Self::UniformList(handle) => handle
                .0
                .borrow()
                .last_item_size
                .map_or_else(Size::default, |size| size.item),
        }
    }

    fn set_position(&self, orientation: ScrollbarOrientation, position: Pixels) {
        let mut offset = self.offset();
        match orientation {
            ScrollbarOrientation::Horizontal => offset.x = -position,
            ScrollbarOrientation::Vertical => offset.y = -position,
        }
        match self {
            Self::Scroll(handle) => handle.set_offset(offset),
            Self::List(state) => state.set_offset_from_scrollbar(offset),
            Self::UniformList(handle) => handle.0.borrow().base_handle.set_offset(offset),
        }
    }

    fn position(&self, orientation: ScrollbarOrientation) -> Pixels {
        let offset = self.offset();
        -orientation.coordinate(offset)
    }

    fn max_position(&self, orientation: ScrollbarOrientation) -> Pixels {
        orientation.coordinate(self.max_offset()).max(Pixels::ZERO)
    }

    fn viewport_length(&self, orientation: ScrollbarOrientation) -> Pixels {
        orientation.length(self.viewport_size()).max(Pixels::ZERO)
    }

    fn set_clamped_position(&self, orientation: ScrollbarOrientation, position: Pixels) {
        self.set_position(
            orientation,
            position.clamp(Pixels::ZERO, self.max_position(orientation)),
        );
    }

    fn scroll_by(&self, orientation: ScrollbarOrientation, delta: Pixels) {
        self.set_clamped_position(orientation, self.position(orientation) + delta);
    }

    fn drag_started(&self) {
        if let Self::List(state) = self {
            state.scrollbar_drag_started();
        }
    }

    fn drag_ended(&self) {
        if let Self::List(state) = self {
            state.scrollbar_drag_ended();
        }
    }
}

/// Semantic thumb states and internal thumb geometry.
#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct ScrollbarAppearance {
    pub thumb: Background,
    pub hover_thumb: Background,
    pub dragging_thumb: Background,
    pub focus_ring: gpui::Hsla,
    pub thumb_radius: Pixels,
    pub min_thumb_length: Pixels,
}

impl Default for ScrollbarAppearance {
    fn default() -> Self {
        Self {
            thumb: gpui::black().opacity(0.32).into(),
            hover_thumb: gpui::black().opacity(0.42).into(),
            dragging_thumb: gpui::black().opacity(0.58).into(),
            focus_ring: gpui::blue().opacity(0.55),
            thumb_radius: px(999.),
            min_thumb_length: px(24.),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    pointer_in_thumb: Pixels,
}

/// Lightweight persistent pointer-capture state for a scrollbar.
#[derive(Clone, Default)]
pub struct ScrollbarState {
    capture: Rc<Cell<Option<HitboxId>>>,
    drag: Rc<Cell<Option<DragState>>>,
    hovered: Rc<Cell<bool>>,
}

impl ScrollbarState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.get().is_some()
    }
}

/// A visual scrollbar attached to an existing GPUI scroll model.
#[derive(IntoElement)]
pub struct Scrollbar {
    id: ElementId,
    state: ScrollbarState,
    source: ScrollbarSource,
    orientation: ScrollbarOrientation,
    appearance: ScrollbarAppearance,
    auto_hide: Option<bool>,
    style: StyleRefinement,
}

impl Scrollbar {
    pub fn vertical(
        id: impl Into<ElementId>,
        state: &ScrollbarState,
        source: impl Into<ScrollbarSource>,
    ) -> Self {
        Self::new(id, state, source, ScrollbarOrientation::Vertical)
    }

    pub fn horizontal(
        id: impl Into<ElementId>,
        state: &ScrollbarState,
        source: impl Into<ScrollbarSource>,
    ) -> Self {
        Self::new(id, state, source, ScrollbarOrientation::Horizontal)
    }

    pub fn new(
        id: impl Into<ElementId>,
        state: &ScrollbarState,
        source: impl Into<ScrollbarSource>,
        orientation: ScrollbarOrientation,
    ) -> Self {
        let style = match orientation {
            ScrollbarOrientation::Vertical => StyleRefinement::default()
                .relative()
                .w(px(10.))
                .h_full()
                .p(px(2.)),
            ScrollbarOrientation::Horizontal => StyleRefinement::default()
                .relative()
                .w_full()
                .h(px(10.))
                .p(px(2.)),
        }
        .rounded_full()
        .border_2()
        .border_color(transparent_black());
        Self {
            id: id.into(),
            state: state.clone(),
            source: source.into(),
            orientation,
            appearance: ScrollbarAppearance::default(),
            auto_hide: None,
            style,
        }
    }

    pub fn appearance(mut self, appearance: ScrollbarAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn auto_hide(mut self, auto_hide: bool) -> Self {
        self.auto_hide = Some(auto_hide);
        self
    }
}

impl RenderOnce for Scrollbar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let max = self.source.max_position(self.orientation);
        let position = self
            .source
            .position(self.orientation)
            .clamp(Pixels::ZERO, max);
        let scrollable = max > Pixels::ZERO;
        let auto_hide = self
            .auto_hide
            .unwrap_or_else(|| cx.should_auto_hide_scrollbars());
        let keyboard_source = self.source.clone();
        let increment_source = self.source.clone();
        let decrement_source = self.source.clone();
        let orientation = self.orientation;
        let page = self.source.viewport_length(orientation).max(px(24.));
        let focus_ring = self.appearance.focus_ring;

        let mut element = div()
            .id(self.id)
            .cursor(gpui::CursorStyle::Arrow)
            .debug_selector(|| "uic-scrollbar".to_string())
            .focusable()
            .tab_stop(scrollable)
            .role(Role::ScrollBar)
            .aria_orientation(self.orientation.accessibility())
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(f64::from(max))
            .aria_numeric_value(f64::from(position))
            .aria_numeric_value_step(f64::from(page))
            .on_key_down(move |event, window, cx| {
                if adjust_from_key(
                    &keyboard_source,
                    orientation,
                    page,
                    event.keystroke.key.as_str(),
                ) {
                    window.refresh();
                    cx.stop_propagation();
                }
            })
            .on_a11y_action(AccessibleAction::Increment, move |_, window, _| {
                increment_source.scroll_by(orientation, page);
                window.refresh();
            })
            .on_a11y_action(AccessibleAction::Decrement, move |_, window, _| {
                decrement_source.scroll_by(orientation, -page);
                window.refresh();
            })
            .child(ScrollbarInteraction {
                state: self.state,
                source: self.source,
                orientation: self.orientation,
                appearance: self.appearance,
                auto_hide,
                style: StyleRefinement::default().absolute().inset_0(),
            });
        element.style().refine(&self.style);
        element.focus_visible(move |style| style.border_color(focus_ring))
    }
}

impl Styled for Scrollbar {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

struct ScrollbarInteraction {
    state: ScrollbarState,
    source: ScrollbarSource,
    orientation: ScrollbarOrientation,
    appearance: ScrollbarAppearance,
    auto_hide: bool,
    style: StyleRefinement,
}

impl IntoElement for ScrollbarInteraction {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ScrollbarInteraction {
    type RequestLayoutState = Style;
    type PrepaintState = Hitbox;

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
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.refine(&self.style);
        let layout_id = window.request_layout(style.clone(), [], cx);
        (layout_id, style)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Style,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        if let Some(previous) = self.state.capture.get()
            && window.captured_hitbox() == Some(previous)
        {
            window.capture_pointer(hitbox.id);
        }
        self.state.capture.set(Some(hitbox.id));
        hitbox
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        style: &mut Style,
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let source = self.source.clone();
        let down_source = source.clone();
        let move_source = source.clone();
        let up_source = source.clone();
        let orientation = self.orientation;
        let min_thumb = self.appearance.min_thumb_length;
        let down_hitbox = hitbox.clone();
        let move_hitbox = hitbox.clone();
        let up_hitbox = hitbox.clone();
        let down_state = self.state.clone();
        let move_state = self.state.clone();
        let up_state = self.state.clone();
        let hover_state = self.state.clone();
        let dragging = self.state.is_dragging();
        let hovered = hitbox.is_hovered(window);
        self.state.hovered.set(hovered);
        let geometry = geometry_for(&source, orientation, bounds, min_thumb);
        let show_thumb = geometry.is_some() && (!self.auto_hide || hovered || dragging);
        let thumb_background = if dragging {
            self.appearance.dragging_thumb
        } else if hovered {
            self.appearance.hover_thumb
        } else {
            self.appearance.thumb
        };
        let thumb_radius = self.appearance.thumb_radius;

        style.paint(bounds, window, cx, move |window, _| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || event.button != MouseButton::Left
                    || !down_hitbox.is_hovered(window)
                {
                    return;
                }
                let Some(geometry) =
                    geometry_for(&down_source, orientation, down_hitbox.bounds, min_thumb)
                else {
                    return;
                };
                let pointer = orientation.coordinate(event.position);
                let track_origin = orientation.coordinate(down_hitbox.bounds.origin);
                let local_pointer = pointer - track_origin;
                if local_pointer >= geometry.start
                    && local_pointer <= geometry.start + geometry.length
                {
                    down_source.drag_started();
                    down_state.drag.set(Some(DragState {
                        pointer_in_thumb: local_pointer - geometry.start,
                    }));
                    window.capture_pointer(down_hitbox.id);
                } else {
                    let delta = if local_pointer < geometry.start {
                        -geometry.viewport
                    } else {
                        geometry.viewport
                    };
                    down_source.scroll_by(orientation, delta);
                }
                window.refresh();
                cx.stop_propagation();
            });

            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                let hovered = move_hitbox.is_hovered(window);
                if hover_state.hovered.replace(hovered) != hovered {
                    window.refresh();
                }
                if phase == DispatchPhase::Capture
                    && event.dragging()
                    && window.captured_hitbox() == Some(move_hitbox.id)
                    && let Some(drag) = move_state.drag.get()
                    && let Some(geometry) =
                        geometry_for(&move_source, orientation, move_hitbox.bounds, min_thumb)
                {
                    let pointer = orientation.coordinate(event.position);
                    let origin = orientation.coordinate(move_hitbox.bounds.origin);
                    let start = pointer - origin - drag.pointer_in_thumb;
                    let ratio = if geometry.travel > Pixels::ZERO {
                        (start / geometry.travel).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    move_source.set_clamped_position(orientation, geometry.max_position * ratio);
                    window.refresh();
                    cx.stop_propagation();
                }
            });

            window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture
                    && event.button == MouseButton::Left
                    && window.captured_hitbox() == Some(up_hitbox.id)
                    && up_state.drag.take().is_some()
                {
                    up_source.drag_ended();
                    window.refresh();
                    cx.stop_propagation();
                }
            });

            if show_thumb && let Some(geometry) = geometry {
                let thumb_bounds = geometry.thumb_bounds(bounds, orientation);
                let radius = thumb_radius
                    .min(thumb_bounds.size.width / 2.)
                    .min(thumb_bounds.size.height / 2.);
                window.paint_quad(fill(thumb_bounds, thumb_background).corner_radii(radius));
            }
        });
    }
}

impl Styled for ScrollbarInteraction {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

#[derive(Clone, Copy, Debug)]
struct ThumbGeometry {
    start: Pixels,
    length: Pixels,
    travel: Pixels,
    viewport: Pixels,
    max_position: Pixels,
}

impl ThumbGeometry {
    fn thumb_bounds(
        self,
        track: Bounds<Pixels>,
        orientation: ScrollbarOrientation,
    ) -> Bounds<Pixels> {
        match orientation {
            ScrollbarOrientation::Horizontal => Bounds {
                origin: point(track.origin.x + self.start, track.origin.y),
                size: size(self.length, track.size.height),
            },
            ScrollbarOrientation::Vertical => Bounds {
                origin: point(track.origin.x, track.origin.y + self.start),
                size: size(track.size.width, self.length),
            },
        }
    }
}

fn geometry_for(
    source: &ScrollbarSource,
    orientation: ScrollbarOrientation,
    bounds: Bounds<Pixels>,
    min_thumb: Pixels,
) -> Option<ThumbGeometry> {
    thumb_geometry(
        orientation.length(bounds.size),
        source.viewport_length(orientation),
        source.position(orientation),
        source.max_position(orientation),
        min_thumb,
    )
}

fn thumb_geometry(
    track_length: Pixels,
    viewport: Pixels,
    position: Pixels,
    max_position: Pixels,
    min_thumb: Pixels,
) -> Option<ThumbGeometry> {
    if track_length <= Pixels::ZERO || max_position <= Pixels::ZERO {
        return None;
    }
    let viewport = if viewport > Pixels::ZERO {
        viewport
    } else {
        track_length
    };
    let content = viewport + max_position;
    let length =
        (track_length * (viewport / content)).clamp(min_thumb.min(track_length), track_length);
    let travel = (track_length - length).max(Pixels::ZERO);
    let progress = (position / max_position).clamp(0.0, 1.0);
    Some(ThumbGeometry {
        start: travel * progress,
        length,
        travel,
        viewport,
        max_position,
    })
}

fn adjust_from_key(
    source: &ScrollbarSource,
    orientation: ScrollbarOrientation,
    page: Pixels,
    key: &str,
) -> bool {
    let line = px(40.);
    let delta = match (orientation, key) {
        (ScrollbarOrientation::Horizontal, "left") | (ScrollbarOrientation::Vertical, "up") => {
            Some(-line)
        }
        (ScrollbarOrientation::Horizontal, "right") | (ScrollbarOrientation::Vertical, "down") => {
            Some(line)
        }
        (_, "pageup") => Some(-page),
        (_, "pagedown") => Some(page),
        (_, "home") => {
            source.set_clamped_position(orientation, Pixels::ZERO);
            return true;
        }
        (_, "end") => {
            source.set_clamped_position(orientation, source.max_position(orientation));
            return true;
        }
        _ => None,
    };
    if let Some(delta) = delta {
        source.scroll_by(orientation, delta);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Modifiers, Render, TestAppContext, VisualTestContext};

    use super::*;

    struct TestScrollbar {
        scroll: ScrollHandle,
        scrollbar: ScrollbarState,
    }

    impl Render for TestScrollbar {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(300.))
                .h(px(220.))
                .flex()
                .child(
                    div()
                        .id("test-scroll-content")
                        .flex_1()
                        .h_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .children((0..30).map(|index| {
                            div().h(px(32.)).flex_none().child(format!("Row {index}"))
                        })),
                )
                .child(
                    Scrollbar::vertical("test-scrollbar", &self.scrollbar, &self.scroll)
                        .auto_hide(false),
                )
        }
    }

    #[test]
    fn thumb_geometry_tracks_scroll_progress_and_respects_minimum_length() {
        let middle = thumb_geometry(px(200.), px(200.), px(400.), px(800.), px(24.))
            .expect("scrollable content should produce a thumb");
        assert_eq!(middle.length, px(40.));
        assert_eq!(middle.start, px(80.));

        let near_end = thumb_geometry(px(100.), px(10.), px(990.), px(990.), px(24.))
            .expect("scrollable content should produce a thumb");
        assert_eq!(near_end.length, px(24.));
        assert_eq!(near_end.start, px(76.));

        assert!(thumb_geometry(px(200.), px(200.), px(0.), px(0.), px(24.)).is_none());
    }

    #[gpui::test]
    fn dragging_the_thumb_updates_the_attached_scroll_handle(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(320.), px(240.)), |_, _| TestScrollbar {
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
            window.draw(cx).clear();
        });

        let track = visual
            .debug_bounds("uic-scrollbar")
            .expect("scrollbar should be rendered");
        let start = point(track.center().x, track.top() + px(8.));
        let end = point(track.center().x, track.bottom() - px(8.));
        visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

        window
            .update(&mut visual.cx, |view, _, _| {
                assert!(view.scroll.offset().y < px(-500.));
                assert!(!view.scrollbar.is_dragging());
            })
            .unwrap();
    }
}
