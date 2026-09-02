mod interaction;
mod state;

use std::{rc::Rc, time::Instant};

use gpui::{
    AnyElement, App, Bounds, CursorStyle, ElementId, Entity, FontWeight, IntoElement, Pixels,
    Point, RenderOnce, SharedString, StyleRefinement, Styled, Window, div, point, prelude::*, px,
    rgb, size,
};
use gpui_effects::{StickyShape, paint_sticky_shapes};

use interaction::{BadgeInteraction, DragPhase};
pub use state::BadgeState;

type DismissCallback = Rc<dyn Fn(&mut Window, &mut App)>;

enum BadgeContent {
    Count(u64),
    Text(SharedString),
    Dot,
}

/// A value or status marker positioned over another element.
#[derive(IntoElement)]
pub struct Badge {
    id: ElementId,
    child: AnyElement,
    content: BadgeContent,
    max: u64,
    show_zero: bool,
    hidden: bool,
    offset: Point<Pixels>,
    dismiss_state: Option<Entity<BadgeState>>,
    dismiss_threshold: Pixels,
    on_dismiss: Option<DismissCallback>,
    style: StyleRefinement,
}

impl Badge {
    pub fn new(id: impl Into<ElementId>, child: impl IntoElement) -> Self {
        Self {
            id: id.into(),
            child: child.into_any_element(),
            content: BadgeContent::Count(0),
            max: 99,
            show_zero: false,
            hidden: false,
            offset: Point::default(),
            dismiss_state: None,
            dismiss_threshold: px(72.),
            on_dismiss: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn count(mut self, count: u64) -> Self {
        self.content = BadgeContent::Count(count);
        self
    }

    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.content = BadgeContent::Text(text.into());
        self
    }

    pub fn dot(mut self) -> Self {
        self.content = BadgeContent::Dot;
        self
    }

    pub fn max(mut self, max: u64) -> Self {
        self.max = max;
        self
    }

    pub fn show_zero(mut self, show_zero: bool) -> Self {
        self.show_zero = show_zero;
        self
    }

    pub fn hidden(mut self, hidden: bool) -> Self {
        self.hidden = hidden;
        self
    }

    /// Offsets the badge from its default top-right position.
    pub fn offset(mut self, offset: Point<Pixels>) -> Self {
        self.offset = offset;
        self
    }

    /// Enables direct, interruptible drag-to-dismiss interaction.
    pub fn dismissible(mut self, state: &Entity<BadgeState>) -> Self {
        self.dismiss_state = Some(state.clone());
        self
    }

    pub fn dismiss_threshold(mut self, threshold: Pixels) -> Self {
        self.dismiss_threshold = threshold.max(px(1.));
        self
    }

    pub fn on_dismiss(mut self, callback: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(callback));
        self
    }
}

impl RenderOnce for Badge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_dot = matches!(self.content, BadgeContent::Dot);
        let label = match self.content {
            BadgeContent::Count(0) if !self.show_zero => None,
            BadgeContent::Count(count) if count > self.max => Some(format!("{}+", self.max)),
            BadgeContent::Count(count) => Some(count.to_string()),
            BadgeContent::Text(text) => Some(text.to_string()),
            BadgeContent::Dot => Some(String::new()),
        };

        let visual = self
            .dismiss_state
            .as_ref()
            .map(|state| state.read(cx).visual(Instant::now()));
        if visual.is_some_and(|visual| visual.active_motion) {
            window.request_animation_frame();
        }
        if let (Some(state), Some((motion_id, _))) = (
            self.dismiss_state.as_ref(),
            visual.and_then(|visual| visual.completion),
        ) {
            let state = state.clone();
            let callback = self.on_dismiss.clone();
            window.on_next_frame(move |window, cx| {
                let dismissed = state.update(cx, |state, cx| state.finish_motion(motion_id, cx));
                if dismissed && let Some(callback) = callback {
                    callback(window, cx);
                }
            });
        }

        let dismissed = self
            .dismiss_state
            .as_ref()
            .is_some_and(|state| state.read(cx).is_dismissed());
        let show_badge = !self.hidden && !dismissed && label.is_some();
        let offset = visual.map_or(Point::default(), |visual| visual.offset);
        let opacity = visual.map_or(1.0, |visual| visual.opacity);
        let dismissible = self.dismiss_state.is_some();
        let dismiss_threshold = self.dismiss_threshold;
        let tether_background = self
            .style
            .background
            .as_ref()
            .and_then(|fill| fill.color())
            .and_then(|background| background.as_solid())
            .unwrap_or_else(|| rgb(0xff4d4f).into());

        let indicator = show_badge.then(|| {
            let mut indicator = div()
                .id(self.id)
                .debug_selector(|| "uic-badge".to_string())
                .flex()
                .items_center()
                .justify_center()
                .flex_none()
                .min_w(px(20.))
                .h(px(20.))
                .px(px(6.))
                .rounded_full()
                .bg(rgb(0xff4d4f))
                .text_color(rgb(0xffffff))
                .text_size(px(11.))
                .line_height(px(20.))
                .font_weight(FontWeight::SEMIBOLD)
                .shadow_sm()
                .when(is_dot, |indicator| {
                    indicator
                        .size(px(8.))
                        .min_w(px(8.))
                        .p_0()
                        .line_height(px(8.))
                })
                .children(label.filter(|label| !label.is_empty()));
            indicator.style().refine(&self.style);

            if let Some(state) = self.dismiss_state {
                let capture = state.read(cx).capture();
                let dragging = state.read(cx).is_dragging();
                let threshold = self.dismiss_threshold;
                let interaction_state = state.clone();
                indicator = indicator
                    .cursor(if dragging {
                        CursorStyle::ClosedHand
                    } else {
                        CursorStyle::OpenHand
                    })
                    .child(
                        BadgeInteraction::new(capture, move |position, phase, _, cx| {
                            interaction_state.update(cx, |state, cx| match phase {
                                DragPhase::Start => state.start_drag(position, cx),
                                DragPhase::Move => state.drag_to(position, cx),
                                DragPhase::End => state.end_drag(position, threshold, cx),
                            });
                        })
                        .absolute()
                        .inset_0(),
                    );
            }
            let tethered = dismissible
                && point_length(offset) > 0.5
                && point_length(offset) < f32::from(dismiss_threshold)
                && !visual.is_some_and(|visual| visual.dismissing);
            div()
                .relative()
                .flex_none()
                .left(offset.x)
                .top(offset.y)
                .opacity(opacity)
                .when(tethered, move |motion| {
                    motion.on_paint_before_children(move |bounds, _, window, _| {
                        paint_elastic_tether(
                            bounds,
                            offset,
                            dismiss_threshold,
                            tether_background,
                            window,
                        );
                    })
                })
                .child(indicator)
        });

        div()
            .relative()
            .flex()
            .flex_none()
            .child(self.child)
            .children(indicator.map(|indicator| {
                div()
                    .absolute()
                    .top(self.offset.y)
                    .right(px(0.) - self.offset.x)
                    .size(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(indicator)
            }))
    }
}

impl Styled for Badge {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn paint_elastic_tether(
    dragged_bounds: Bounds<Pixels>,
    offset: Point<Pixels>,
    threshold: Pixels,
    background: gpui::Hsla,
    window: &mut Window,
) {
    let distance = point_length(offset);
    if distance <= 3.0 || distance >= f32::from(threshold) {
        return;
    }

    let dragged_center = point(
        dragged_bounds.origin.x + dragged_bounds.size.width / 2.,
        dragged_bounds.origin.y + dragged_bounds.size.height / 2.,
    );
    let anchor_center = point(dragged_center.x - offset.x, dragged_center.y - offset.y);
    let bubble_radius = dragged_bounds.size.height / 2.;
    let tension = (distance / f32::from(threshold)).clamp(0.0, 1.0);
    let anchor_radius = bubble_radius * (1.0 - tension * 0.52).max(0.48);
    let target_scale = 1.0 + tension * 0.18;
    let target_size = size(
        dragged_bounds.size.width * target_scale,
        dragged_bounds.size.height * target_scale,
    );
    let target_radius = bubble_radius * target_scale;

    let padding = px(3.);
    let anchor_half = point(anchor_radius, anchor_radius);
    let dragged_half = point(target_size.width / 2., target_size.height / 2.);
    let field_origin = point(
        (anchor_center.x - anchor_half.x).min(dragged_center.x - dragged_half.x) - padding,
        (anchor_center.y - anchor_half.y).min(dragged_center.y - dragged_half.y) - padding,
    );
    let field_end = point(
        (anchor_center.x + anchor_half.x).max(dragged_center.x + dragged_half.x) + padding,
        (anchor_center.y + anchor_half.y).max(dragged_center.y + dragged_half.y) + padding,
    );
    let field_bounds = Bounds::new(
        field_origin,
        size(field_end.x - field_origin.x, field_end.y - field_origin.y),
    );
    let anchor = StickyShape::new(
        anchor_center - field_origin,
        size(anchor_radius * 2., anchor_radius * 2.),
        anchor_radius,
    );
    let target = StickyShape::new(dragged_center - field_origin, target_size, target_radius);
    let _ = paint_sticky_shapes(window, field_bounds, anchor, target, tension, background);
}

fn point_length(value: Point<Pixels>) -> f32 {
    f32::from(value.x).hypot(f32::from(value.y))
}
