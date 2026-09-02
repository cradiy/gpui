use std::{rc::Rc, time::Duration};

use gpui::{
    AccessibleAction, Animation, AnimationExt as _, AnyElement, App, Background, Bounds, ElementId,
    IntoElement, Pixels, RenderOnce, Role, SharedString, StyleRefinement, Styled, Window, div,
    fill, point, prelude::*, px, relative, size, transparent_black,
};

use super::SwitchAppearance;

type ChangeCallback = Rc<dyn Fn(bool, &mut Window, &mut App)>;

/// A controlled boolean switch. Compose labels outside the track when needed.
#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    checked: bool,
    disabled: bool,
    label: Option<SharedString>,
    checked_content: Option<AnyElement>,
    unchecked_content: Option<AnyElement>,
    on_change: Option<ChangeCallback>,
    appearance: SwitchAppearance,
    animation_duration: Duration,
    style: StyleRefinement,
}

impl Switch {
    pub fn new(id: impl Into<ElementId>, checked: bool) -> Self {
        Self {
            id: id.into(),
            checked,
            disabled: false,
            label: None,
            checked_content: None,
            unchecked_content: None,
            on_change: None,
            appearance: SwitchAppearance::default(),
            animation_duration: Duration::from_millis(220),
            style: StyleRefinement::default()
                .w(px(44.))
                .h(px(24.))
                .p(px(2.))
                .rounded_full()
                .overflow_hidden()
                .border_1()
                .border_color(gpui::transparent_black())
                .text_size(px(12.)),
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Sets the content rendered inside the track when the switch is checked.
    ///
    /// Use [`Styled`] width methods to leave enough room for longer content.
    pub fn checked_content(mut self, content: impl IntoElement) -> Self {
        self.checked_content = Some(content.into_any_element());
        self
    }

    /// Sets the content rendered inside the track when the switch is unchecked.
    ///
    /// Use [`Styled`] width methods to leave enough room for longer content.
    pub fn unchecked_content(mut self, content: impl IntoElement) -> Self {
        self.unchecked_content = Some(content.into_any_element());
        self
    }

    pub fn on_change(mut self, callback: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(callback));
        self
    }

    pub fn appearance(mut self, appearance: SwitchAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Sets the state transition duration. Use [`Duration::ZERO`] to disable animation.
    pub fn animation_duration(mut self, duration: Duration) -> Self {
        self.animation_duration = duration;
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let next_value = !self.checked;
        let group_name = SharedString::from(format!("uic-switch-{}", self.id));
        let motion = switch_motion(
            self.checked,
            self.animation_duration,
            self.appearance.thumb_size,
            self.appearance.thumb,
        );
        let track = switch_track(
            self.checked,
            self.animation_duration,
            self.appearance.on_track,
            self.appearance.off_track,
            self.appearance.hover_on_track,
            self.appearance.hover_off_track,
            self.appearance.thumb_size,
            group_name.clone(),
            self.disabled,
        );
        let blocks = switch_blocks(
            self.checked_content,
            self.unchecked_content,
            self.checked,
            self.animation_duration,
            self.appearance.thumb_size,
            self.appearance.on_content,
            self.appearance.off_content,
        );
        let visual = div()
            .size_full()
            .relative()
            .child(div().absolute().inset_0().child(motion));
        let click_callback = self.on_change.clone();
        let key_callback = self.on_change.clone();
        let action_callback = self.on_change.clone();

        let mut element = div()
            .id(self.id)
            .debug_selector(|| "uic-switch".to_string())
            .group(group_name)
            .focusable()
            .tab_stop(!self.disabled)
            .role(Role::Switch)
            .aria_toggled(self.checked.into())
            .when_some(self.label, |this, label| this.aria_label(label))
            .flex()
            .items_center()
            .relative()
            .bg(self.appearance.off_track)
            .opacity(if self.disabled {
                self.appearance.disabled_opacity
            } else {
                1.0
            })
            .child(track)
            .child(blocks)
            .child(visual);
        element.style().refine(&self.style);
        if !self.disabled {
            element = element
                .cursor_pointer()
                .when_some(click_callback, |this, callback| {
                    this.on_click(move |_, window, cx| callback(next_value, window, cx))
                })
                .when_some(key_callback, |this, callback| {
                    this.on_key_down(move |event, window, cx| {
                        if event.keystroke.key == "space" {
                            callback(next_value, window, cx);
                            cx.stop_propagation();
                        }
                    })
                })
                .when_some(action_callback, |this, callback| {
                    this.on_a11y_action(AccessibleAction::Click, move |_, window, cx| {
                        callback(next_value, window, cx)
                    })
                });
        }
        element.focus_visible(move |style| style.border_color(self.appearance.focus_ring))
    }
}

impl Styled for Switch {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

#[allow(clippy::too_many_arguments)]
fn switch_blocks(
    checked_content: Option<AnyElement>,
    unchecked_content: Option<AnyElement>,
    checked: bool,
    duration: Duration,
    thumb_size: Pixels,
    on_color: gpui::Hsla,
    off_color: gpui::Hsla,
) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .children([animated_switch_block(
            checked_content,
            true,
            checked,
            duration,
            thumb_size,
            on_color,
        )])
        .children([animated_switch_block(
            unchecked_content,
            false,
            checked,
            duration,
            thumb_size,
            off_color,
        )])
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn animated_switch_block(
    content: Option<AnyElement>,
    checked_block: bool,
    checked: bool,
    duration: Duration,
    thumb_size: Pixels,
    color: gpui::Hsla,
) -> AnyElement {
    let content = content.map(|content| {
        div()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .flex()
            .items_center()
            .justify_center()
            .text_color(color)
            .when(checked_block, |content| content.pr(thumb_size))
            .when(!checked_block, |content| content.pl(thumb_size))
            .child(content)
    });
    let layer = div()
        .absolute()
        .top_0()
        .h_full()
        .min_h_0()
        .max_h_full()
        .overflow_hidden()
        .children(content)
        .when(checked_block, |block| block.left_0())
        .when(!checked_block, |block| block.right_0());
    let width = move |progress: f32| {
        relative(if checked_block {
            progress
        } else {
            1.0 - progress
        })
    };
    if duration.is_zero() {
        let progress = if checked { 1.0 } else { 0.0 };
        layer.w(width(progress)).into_any_element()
    } else {
        let animation_id = match (checked_block, checked) {
            (true, true) => "uic-switch-on-block-expand",
            (true, false) => "uic-switch-on-block-collapse",
            (false, true) => "uic-switch-off-block-collapse",
            (false, false) => "uic-switch-off-block-expand",
        };
        layer
            .with_animation(
                animation_id,
                Animation::new(duration),
                move |layer, phase| {
                    let progress = split_motion_progress(phase);
                    let progress = if checked { progress } else { 1.0 - progress };
                    layer.w(width(progress))
                },
            )
            .into_any_element()
    }
}

#[allow(clippy::too_many_arguments)]
fn switch_track(
    checked: bool,
    duration: Duration,
    on_background: Background,
    off_background: Background,
    hover_on_background: Background,
    hover_off_background: Background,
    thumb_size: Pixels,
    group_name: SharedString,
    disabled: bool,
) -> AnyElement {
    let layer = div()
        .absolute()
        .inset_0()
        .rounded_full()
        .bg(off_background)
        .when(!disabled, move |layer| {
            layer.group_hover(group_name, move |style| {
                style
                    .bg(hover_off_background)
                    // This transparent border is a paint-time hover marker. The layer has no
                    // border width, so it does not alter the switch's geometry or appearance.
                    .border_color(transparent_black())
            })
        });
    let paint_progress = move |layer: gpui::Div, progress: f32| {
        layer.on_paint_before_children(move |bounds, style, window, _| {
            let hovered = style.border_color.is_some();
            let on_background = if hovered {
                hover_on_background
            } else {
                on_background
            };
            let off_background = if hovered {
                hover_off_background
            } else {
                off_background
            };
            let radius = bounds.size.height / 2.;
            let inset = ((bounds.size.height - thumb_size) / 2.).max(Pixels::ZERO);
            let travel = (bounds.size.width - thumb_size - inset * 2.).max(Pixels::ZERO);
            let split_x = if progress <= 0.0 {
                bounds.origin.x
            } else if progress >= 1.0 {
                bounds.bottom_right().x
            } else {
                bounds.origin.x + inset + thumb_size / 2. + travel * progress
            };

            if progress > 0.0 {
                let on_bounds = Bounds::new(
                    bounds.origin,
                    size(split_x - bounds.origin.x, bounds.size.height),
                );
                window.paint_quad(fill(on_bounds, on_background).corner_radii(radius));
            }
            if progress < 1.0 {
                let off_bounds = Bounds::new(
                    point(split_x, bounds.origin.y),
                    size(bounds.bottom_right().x - split_x, bounds.size.height),
                );
                window.paint_quad(fill(off_bounds, off_background).corner_radii(radius));
            }
        })
    };

    if duration.is_zero() {
        paint_progress(layer, if checked { 1.0 } else { 0.0 }).into_any_element()
    } else {
        layer
            .with_animation(
                if checked {
                    "uic-switch-track-on"
                } else {
                    "uic-switch-track-off"
                },
                Animation::new(duration),
                move |layer, phase| {
                    let progress = split_motion_progress(phase);
                    paint_progress(layer, if checked { progress } else { 1.0 - progress })
                },
            )
            .into_any_element()
    }
}

fn switch_motion(
    checked: bool,
    duration: Duration,
    thumb_size: Pixels,
    thumb: Background,
) -> AnyElement {
    let position_thumb = move |track: gpui::Div, position: f32, stretch: f32| {
        let stretch = thumb_size * 0.30 * stretch;
        track
            .flex()
            .items_center()
            .child(div().flex_basis(px(0.)).flex_grow(position).flex_shrink_1())
            .child(
                div()
                    .flex_none()
                    .w(thumb_size + stretch)
                    .h(thumb_size)
                    .rounded_full()
                    .bg(thumb)
                    .shadow_sm(),
            )
            .child(
                div()
                    .flex_basis(px(0.))
                    .flex_grow(1.0 - position)
                    .flex_shrink_1(),
            )
    };
    let track = div().size_full();
    if duration.is_zero() {
        position_thumb(track, if checked { 1.0 } else { 0.0 }, 0.0).into_any_element()
    } else {
        track
            .with_animation(
                if checked {
                    "uic-switch-thumb-on"
                } else {
                    "uic-switch-thumb-off"
                },
                Animation::new(duration),
                move |track, phase| {
                    let progress = split_motion_progress(phase);
                    position_thumb(
                        track,
                        if checked { progress } else { 1.0 - progress },
                        thumb_stretch(phase),
                    )
                },
            )
            .into_any_element()
    }
}

fn split_motion_progress(phase: f32) -> f32 {
    const MIDPOINT_POSITION: f32 = 0.30;
    const MIDPOINT_TANGENT: f32 = 0.35;

    let phase = phase.clamp(0.0, 1.0);
    if phase <= 0.5 {
        cubic_hermite(phase * 2.0, 0.0, MIDPOINT_POSITION, 0.0, MIDPOINT_TANGENT)
    } else {
        cubic_hermite(
            (phase - 0.5) * 2.0,
            MIDPOINT_POSITION,
            1.0,
            MIDPOINT_TANGENT,
            0.0,
        )
    }
}

fn thumb_stretch(phase: f32) -> f32 {
    if phase <= 0.5 {
        smoothstep((phase * 2.0).clamp(0.0, 1.0))
    } else {
        1.0 - smoothstep(((phase - 0.5) / 0.35).clamp(0.0, 1.0))
    }
}

fn cubic_hermite(phase: f32, from: f32, to: f32, from_tangent: f32, to_tangent: f32) -> f32 {
    let phase_2 = phase * phase;
    let phase_3 = phase_2 * phase;
    (2.0 * phase_3 - 3.0 * phase_2 + 1.0) * from
        + (phase_3 - 2.0 * phase_2 + phase) * from_tangent
        + (-2.0 * phase_3 + 3.0 * phase_2) * to
        + (phase_3 - phase_2) * to_tangent
}

fn smoothstep(phase: f32) -> f32 {
    phase * phase * (3.0 - 2.0 * phase)
}
