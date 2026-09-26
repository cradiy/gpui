use gpui::{
    AccessibleAction, App, Background, ColorSpace, CursorStyle, Entity, FocusHandle, Hsla,
    IntoElement, KeyDownEvent, Orientation, Refineable as _, RenderOnce, Role, SharedString,
    StyleRefinement, Styled, Window, black, div, hsla, linear_color_stop, linear_gradient,
    prelude::*, px, relative, transparent_black, transparent_white, white,
};

use super::{
    ColorPickerAppearance, ColorPickerState, Hsva,
    interaction::{CaptureToken, ColorInteraction, InteractionPhase},
};

#[derive(IntoElement)]
pub struct ColorPicker {
    state: Entity<ColorPickerState>,
    appearance: ColorPickerAppearance,
    sv_label: Option<SharedString>,
    hue_label: Option<SharedString>,
    horizontal_hue: bool,
    style: StyleRefinement,
}

impl ColorPicker {
    pub fn new(state: &Entity<ColorPickerState>) -> Self {
        Self {
            state: state.clone(),
            appearance: ColorPickerAppearance::default(),
            sv_label: None,
            hue_label: None,
            horizontal_hue: false,
            style: StyleRefinement::default()
                .w_full()
                .gap(px(14.))
                .p(px(16.))
                .rounded(px(12.))
                .border_1()
                .border_color(hsla(0.52, 0.08, 0.26, 1.0))
                .bg(hsla(0.52, 0.12, 0.11, 1.0)),
        }
    }

    pub fn appearance(mut self, appearance: ColorPickerAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Place the hue slider below the color area.
    pub fn horizontal_hue(mut self, horizontal: bool) -> Self {
        self.horizontal_hue = horizontal;
        self
    }

    pub fn sv_aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.sv_label = Some(label.into());
        self
    }

    pub fn hue_aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.hue_label = Some(label.into());
        self
    }

    fn color_area(
        &self,
        hsva: Hsva,
        capture: CaptureToken,
        focus: FocusHandle,
    ) -> impl IntoElement {
        let appearance = self.appearance;
        let state = self.state.clone();
        let keyboard_state = self.state.clone();
        let label = self.sv_label.clone();
        let hue = Hsva::new(hsva.h, 1.0, 1.0, 1.0).to_rgba();
        let marker_offset = appearance.marker_size * -0.5;

        div()
            .id(("color-picker-sv-control", self.state.entity_id()))
            .relative()
            .debug_selector(|| "color-picker-sv".to_string())
            .track_focus(&focus)
            .tab_stop(true)
            .role(Role::ColorWell)
            .when_some(label, |element, label| element.aria_label(label))
            .aria_value(SharedString::from(format!(
                "{:.0}%, {:.0}%",
                hsva.s * 100.0,
                hsva.v * 100.0
            )))
            .focus_visible(move |style| style.border_2().border_color(appearance.accent))
            .on_key_down(move |event, _, cx| {
                adjust_sv_from_key(&keyboard_state, event, cx);
            })
            .flex_1()
            .h(appearance.area_height)
            .overflow_hidden()
            .cursor(CursorStyle::Crosshair)
            .bg(hue)
            .child(div().absolute().inset_0().bg(two_stop_gradient(
                90.0,
                white(),
                transparent_white(),
            )))
            .child(div().absolute().inset_0().bg(two_stop_gradient(
                180.0,
                transparent_black(),
                black(),
            )))
            .child(
                div()
                    .absolute()
                    .left(relative(hsva.s))
                    .top(relative(1.0 - hsva.v))
                    .ml(marker_offset)
                    .mt(marker_offset)
                    .size(appearance.marker_size)
                    .rounded_full()
                    .border_2()
                    .border_color(appearance.marker)
                    .shadow_sm(),
            )
            .child(
                ColorInteraction::new(capture, focus, move |point, phase, _, cx| {
                    state.update(cx, |state, cx| {
                        state.update_sv(
                            point.x,
                            1.0 - point.y,
                            phase == InteractionPhase::Commit,
                            cx,
                        )
                    });
                })
                .absolute()
                .inset_0(),
            )
    }

    fn hue_track(&self, hsva: Hsva, capture: CaptureToken, focus: FocusHandle) -> impl IntoElement {
        let horizontal = self.horizontal_hue;
        let appearance = self.appearance;
        let state = self.state.clone();
        let keyboard_state = self.state.clone();
        let label = self.hue_label.clone();
        let increment_state = self.state.clone();
        let decrement_state = self.state.clone();
        let colors = [
            hsla(0.0, 1.0, 0.5, 1.0),
            hsla(1.0 / 6.0, 1.0, 0.5, 1.0),
            hsla(2.0 / 6.0, 1.0, 0.5, 1.0),
            hsla(3.0 / 6.0, 1.0, 0.5, 1.0),
            hsla(4.0 / 6.0, 1.0, 0.5, 1.0),
            hsla(5.0 / 6.0, 1.0, 0.5, 1.0),
            hsla(0.0, 1.0, 0.5, 1.0),
        ];
        let mut track = div()
            .id(("color-picker-hue-control", self.state.entity_id()))
            .relative()
            .debug_selector(|| "color-picker-hue".to_string())
            .track_focus(&focus)
            .tab_stop(true)
            .role(Role::Slider)
            .when_some(label, |element, label| element.aria_label(label))
            .aria_numeric_value(f64::from(hsva.h * 360.0))
            .aria_numeric_value_step(1.0)
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(360.0)
            .aria_orientation(if horizontal {
                Orientation::Horizontal
            } else {
                Orientation::Vertical
            })
            .focus_visible(move |style| style.border_2().border_color(appearance.accent))
            .on_key_down(move |event, _, cx| {
                adjust_hue_from_key(&keyboard_state, event, cx);
            })
            .on_a11y_action(AccessibleAction::Increment, move |_, _, cx| {
                adjust_hue(&increment_state, 1.0 / 360.0, cx);
            })
            .on_a11y_action(AccessibleAction::Decrement, move |_, _, cx| {
                adjust_hue(&decrement_state, -1.0 / 360.0, cx);
            })
            .w(appearance.hue_width)
            .h(appearance.area_height)
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_lg()
            .cursor(CursorStyle::ResizeUpDown);
        if horizontal {
            track = track
                .w_full()
                .h(appearance.hue_width)
                .flex_row()
                .cursor(CursorStyle::ResizeLeftRight);
        }
        for pair in colors.windows(2) {
            track = track.child(div().flex_1().bg(two_stop_gradient(
                if horizontal { 90.0 } else { 180.0 },
                pair[0],
                pair[1],
            )));
        }
        track
            .child(
                div()
                    .when(horizontal, |el| el.hidden())
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(relative(hsva.h))
                    .mt(px(-2.0))
                    .h(px(4.0))
                    .border_1()
                    .border_color(black())
                    .bg(appearance.marker),
            )
            .when(horizontal, |el| {
                el.child(
                    div()
                        .absolute()
                        .left(appearance.hue_width / 2.)
                        .right(appearance.hue_width / 2.)
                        .top_0()
                        .bottom_0()
                        .child(
                            div()
                                .absolute()
                                .left(relative(hsva.h))
                                .ml(-appearance.hue_width / 2.)
                                .size(appearance.hue_width)
                                .rounded(appearance.hue_width / 2.)
                                .border_2()
                                .border_color(appearance.marker)
                                .bg(hsla(hsva.h, 1., 0.5, 1.)),
                        ),
                )
            })
            .child(
                ColorInteraction::new(capture, focus, move |point, phase, _, cx| {
                    state.update(cx, |state, cx| {
                        state.update_hue(
                            if horizontal { point.x } else { point.y },
                            phase == InteractionPhase::Commit,
                            cx,
                        )
                    });
                })
                .absolute()
                .inset_0(),
            )
    }
}

impl RenderOnce for ColorPicker {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (hsva, sv_capture, hue_capture, sv_focus, hue_focus) = {
            let state = self.state.read(cx);
            (
                state.hsva(),
                state.sv_capture(),
                state.hue_capture(),
                state.sv_focus(),
                state.hue_focus(),
            )
        };

        let mut element = div()
            .flex()
            .when(self.horizontal_hue, |el| el.flex_col())
            .child(self.color_area(hsva, sv_capture, sv_focus))
            .child(self.hue_track(hsva, hue_capture, hue_focus));
        element.style().refine(&self.style);
        element
    }
}

impl Styled for ColorPicker {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

fn adjust_sv_from_key(state: &Entity<ColorPickerState>, event: &KeyDownEvent, cx: &mut App) {
    let step = if event.keystroke.modifiers.shift {
        0.1
    } else {
        0.01
    };
    let delta = match event.keystroke.key.as_str() {
        "left" => Some((-step, 0.0)),
        "right" => Some((step, 0.0)),
        "up" => Some((0.0, step)),
        "down" => Some((0.0, -step)),
        _ => None,
    };
    if let Some((saturation, value)) = delta {
        state.update(cx, |state, cx| {
            let current = state.hsva();
            state.update_sv(current.s + saturation, current.v + value, true, cx);
        });
        cx.stop_propagation();
    }
}

fn adjust_hue_from_key(state: &Entity<ColorPickerState>, event: &KeyDownEvent, cx: &mut App) {
    let degrees = if event.keystroke.modifiers.shift {
        10.0
    } else {
        1.0
    };
    let delta = match event.keystroke.key.as_str() {
        "left" | "up" => Some(-degrees / 360.0),
        "right" | "down" => Some(degrees / 360.0),
        _ => None,
    };
    if let Some(delta) = delta {
        adjust_hue(state, delta, cx);
        cx.stop_propagation();
    }
}

fn adjust_hue(state: &Entity<ColorPickerState>, delta: f32, cx: &mut App) {
    state.update(cx, |state, cx| {
        state.update_hue(state.hsva().h + delta, true, cx);
    });
}

fn two_stop_gradient(angle: f32, from: impl Into<Hsla>, to: impl Into<Hsla>) -> Background {
    linear_gradient(
        angle,
        linear_color_stop(from, 0.0),
        linear_color_stop(to, 1.0),
    )
    .color_space(ColorSpace::Srgb)
}

#[cfg(test)]
mod tests {
    use gpui::{
        Context, Modifiers, MouseButton, Render, TestAppContext, VisualTestContext, point, rgba,
        size,
    };

    use super::*;

    struct TestPicker {
        state: Entity<ColorPickerState>,
    }

    impl Render for TestPicker {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            ColorPicker::new(&self.state)
        }
    }

    #[gpui::test]
    fn sv_drag_clamps_after_leaving_the_picker(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(640.0), px(420.0)), |_, cx| TestPicker {
            state: cx.new(|cx| ColorPickerState::new(rgba(0xff0000ff), cx)),
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });
        let bounds = visual
            .debug_bounds("color-picker-sv")
            .expect("SV selector should be rendered");
        visual.update(|window, _| {
            assert_eq!(window.captured_hitbox(), None);
        });
        let start = point(
            bounds.left() + bounds.size.width * 0.25,
            bounds.top() + bounds.size.height * 0.25,
        );
        let outside = point(bounds.right() + px(20.0), bounds.bottom());

        visual.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        visual.update(|window, _| {
            assert!(window.captured_hitbox().is_some());
        });
        visual.simulate_mouse_move(outside, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());

        window
            .update(&mut visual.cx, |view, _, cx| {
                let hsva = view.state.read(cx).hsva();
                assert_eq!(hsva.s, 1.0);
                assert_eq!(hsva.v, 0.0);
            })
            .unwrap();
    }

    #[gpui::test]
    fn keyboard_adjustments_follow_the_canvas_axes(cx: &mut TestAppContext) {
        let window = cx.open_window(size(px(640.0), px(420.0)), |_, cx| TestPicker {
            state: cx.new(|cx| ColorPickerState::new(rgba(0xff0000ff), cx)),
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.draw(cx).clear();
        });

        let sv = visual
            .debug_bounds("color-picker-sv")
            .expect("SV selector should be rendered");
        let sv_center = point(
            sv.left() + sv.size.width * 0.5,
            sv.top() + sv.size.height * 0.5,
        );
        visual.simulate_mouse_down(sv_center, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(sv_center, MouseButton::Left, Modifiers::default());
        visual.simulate_keystrokes("right up");

        let hue = visual
            .debug_bounds("color-picker-hue")
            .expect("hue selector should be rendered");
        let hue_center = point(
            hue.left() + hue.size.width * 0.5,
            hue.top() + hue.size.height * 0.5,
        );
        visual.simulate_mouse_down(hue_center, MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(hue_center, MouseButton::Left, Modifiers::default());
        visual.simulate_keystrokes("down");

        window
            .update(&mut visual.cx, |view, _, cx| {
                let hsva = view.state.read(cx).hsva();
                assert!((hsva.s - 0.51).abs() < 1e-5);
                assert!((hsva.v - 0.51).abs() < 1e-5);
                assert!((hsva.h - (0.5 + 1.0 / 360.0)).abs() < 1e-5);
            })
            .unwrap();
    }
}
