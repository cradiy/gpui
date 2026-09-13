use std::{cell::Cell, rc::Rc, time::Instant};

use gpui::{
    AnyElement, App, Bounds, BoxShadow, ElementId, IntoElement, MouseButton, Pixels, RenderOnce,
    Role, SharedString, StyleRefinement, Styled, Window, canvas, div, fill, hsla, point,
    prelude::*, px, rgb,
};
use gpui_effects::{paint_deformed_liquid_glass, paint_liquid_glass};

use super::{GlassSegmentedAppearance, motion::Motion};

type ChangeCallback<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;

struct OptionItem<T> {
    value: T,
    content: AnyElement,
    disabled: bool,
}

/// A controlled, single-selection glass control with an interruptible indicator.
/// Option content stays in layout while the glass moves underneath it. Give each
/// control a stable ID and update the selected value in `on_change`.
/// Arrow keys wrap over enabled options; Home/End select the first/last.
#[derive(IntoElement)]
pub struct GlassSegmentedControl<T: Clone + PartialEq + 'static> {
    id: ElementId,
    selected: T,
    options: Vec<OptionItem<T>>,
    on_change: Option<ChangeCallback<T>>,
    label: Option<SharedString>,
    disabled: bool,
    animated: bool,
    reduced_transparency: bool,
    appearance: GlassSegmentedAppearance,
    style: StyleRefinement,
}

impl<T: Clone + PartialEq + 'static> GlassSegmentedControl<T> {
    pub fn new(id: impl Into<ElementId>, selected: T) -> Self {
        Self {
            id: id.into(),
            selected,
            options: Vec::new(),
            on_change: None,
            label: None,
            disabled: false,
            animated: true,
            reduced_transparency: false,
            appearance: GlassSegmentedAppearance::default(),
            style: StyleRefinement::default()
                .flex()
                .items_center()
                .p(px(4.))
                .gap(px(2.))
                .rounded(px(24.))
                .border_1()
                .border_color(gpui::transparent_black())
                .text_color(rgb(0x344a60))
                .text_size(px(14.))
                .line_height(px(20.)),
        }
    }

    /// Appends content-sized options. Values must be unique within the control.
    #[track_caller]
    pub fn option(mut self, value: T, content: impl IntoElement) -> Self {
        assert!(
            !self.options.iter().any(|option| option.value == value),
            "segmented option values must be unique"
        );
        self.options.push(OptionItem {
            value,
            content: content.into_any_element(),
            disabled: false,
        });
        self
    }

    pub fn disabled_option(self, value: T, content: impl IntoElement) -> Self {
        let mut this = self.option(value, content);
        this.options.last_mut().unwrap().disabled = true;
        this
    }

    pub fn on_change(mut self, callback: impl Fn(T, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(callback));
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Disables movement and press interpolation when false.
    pub fn animated(mut self, animated: bool) -> Self {
        self.animated = animated;
        self
    }

    /// Uses opaque surfaces instead of backdrop sampling.
    pub fn reduced_transparency(mut self, reduced: bool) -> Self {
        self.reduced_transparency = reduced;
        self
    }

    pub fn appearance(mut self, appearance: GlassSegmentedAppearance) -> Self {
        self.appearance = appearance;
        self
    }
}

impl<T: Clone + PartialEq + 'static> Styled for GlassSegmentedControl<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<T: Clone + PartialEq + 'static> RenderOnce for GlassSegmentedControl<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let motion = window.use_keyed_state(self.id.clone(), cx, |_, _| Motion::default());
        let selected_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        let enabled: Vec<T> = self
            .options
            .iter()
            .filter(|o| !o.disabled)
            .map(|o| o.value.clone())
            .collect();
        let interactive = !self.disabled && !enabled.is_empty();
        let key_selected = self.selected.clone();
        let key_callback = self.on_change.clone();
        let appearance = self.appearance;
        let options = self
            .options
            .into_iter()
            .enumerate()
            .map(|(index, option)| {
                let selected = option.value == self.selected;
                let disabled = self.disabled || option.disabled;
                let target = selected_bounds.clone();
                let callback = self.on_change.clone();
                let press = motion.clone();
                let option_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
                let pressed_bounds = option_bounds.clone();
                div()
                    .id(("glass-segment", index))
                    .debug_selector(move || format!("uic-glass-segment-{index}"))
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_none()
                    .px(px(18.))
                    .py(px(8.))
                    .rounded_full()
                    .role(Role::RadioButton)
                    .aria_toggled(selected.into())
                    .when(selected, |element| {
                        element
                            .aria_active_descendant()
                            .text_color(appearance.selected_text)
                    })
                    .when(disabled && !self.disabled, |element| {
                        element.opacity(appearance.disabled_opacity)
                    })
                    .when(!disabled, |element| {
                        element
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                                press.update(cx, |state, cx| {
                                    state.press_focus =
                                        pressed_bounds.get().map_or(point(0.5, 0.5), |bounds| {
                                            let local = event.position - bounds.origin;
                                            point(
                                                (local.x / bounds.size.width.max(px(1.)))
                                                    .clamp(0., 1.),
                                                (local.y / bounds.size.height.max(px(1.)))
                                                    .clamp(0., 1.),
                                            )
                                        });
                                    state.begin_press(Instant::now());
                                    cx.notify();
                                });
                            })
                            .when_some(callback.filter(|_| !selected), |element, callback| {
                                element.on_click(move |_, window, cx| {
                                    callback(option.value.clone(), window, cx)
                                })
                            })
                    })
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                option_bounds.set(Some(bounds));
                                if selected {
                                    target.set(Some(bounds));
                                }
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0()
                        .size_full(),
                    )
                    .child(option.content)
            })
            .collect::<Vec<_>>();
        let release = motion.clone();
        let release_out = motion.clone();
        let hover = motion.clone();
        let pointer = motion.clone();
        let reduced = self.reduced_transparency || !window.supports_backdrop_blur();
        let animated = self.animated;
        let disabled = self.disabled;
        let painted = div().on_paint_before_children(move |bounds, style, window, cx| {
            let corners = style
                .corner_radii
                .to_pixels(window.rem_size())
                .clamp_radii_for_quad_size(bounds.size);
            let mut fallback = appearance.surface.tint;
            fallback.a = 1.;
            if reduced {
                window.paint_quad(fill(bounds, fallback).corner_radii(corners));
            } else {
                paint_liquid_glass(bounds, corners, appearance.surface, window);
            }
            let (visual, pointer) = motion.update(cx, |state, _| {
                if disabled || !window.is_window_active() {
                    state.cancel_press();
                    state.pointer = None;
                }
                let target = selected_bounds
                    .get()
                    .filter(|selected| {
                        selected.size.width > px(0.) && selected.size.height > px(0.)
                    })
                    .map(|selected| Bounds::new(selected.origin - bounds.origin, selected.size));
                (
                    state.sample(target, Instant::now(), animated),
                    state.pointer,
                )
            });
            let Some(visual) = visual else {
                return;
            };
            if visual.moving && window.is_window_active() {
                window.request_animation_frame();
            }
            let mut selected = visual.bounds;
            selected.origin += bounds.origin;
            let selected_corners = corners
                .map(|radius| (*radius - px(4.)).max(px(0.)))
                .clamp_radii_for_quad_size(selected.size);
            window.paint_drop_shadows(
                selected,
                selected_corners,
                &[BoxShadow::new(
                    px(0.),
                    px(2.5 - visual.pressure),
                    hsla(0.6, 0.2, 0.05, 0.1 * (1. - visual.pressure).powi(2)),
                )
                .blur_radius(px(7. + visual.pressure * 5.))],
            );
            let mut optics = appearance.selection;
            optics.highlight *= 1. - visual.pressure * 0.15;
            optics.refraction *= 1. + visual.pressure * 0.15;
            optics.thickness *= 1. + visual.pressure * 0.1;
            optics.tint.a *= 1. - visual.pressure * 0.15;
            if let Some(pointer) = pointer {
                let local = pointer - selected.center();
                optics.light_direction = point(
                    -0.6 + (f32::from(local.x) / f32::from(bounds.size.width).max(1.))
                        .clamp(-0.3, 0.3),
                    -0.8,
                );
            }
            if reduced {
                window.paint_quad(
                    fill(selected, fallback.blend(optics.tint)).corner_radii(selected_corners),
                );
            } else {
                paint_deformed_liquid_glass(
                    selected,
                    selected_corners,
                    optics,
                    visual.deformation,
                    window,
                );
            }
        });
        let mut root = painted
            .id(self.id)
            .debug_selector(|| "uic-glass-segmented".into())
            .focusable()
            .tab_stop(interactive)
            .role(Role::RadioGroup)
            .when_some(self.label, |element, label| element.aria_label(label))
            .on_key_down(move |event, window, cx| {
                if !interactive {
                    return;
                }
                if let Some(value) = next_value(&enabled, &key_selected, &event.keystroke.key) {
                    if value != &key_selected
                        && let Some(callback) = &key_callback
                    {
                        callback(value.clone(), window, cx);
                    }
                    cx.stop_propagation();
                }
            })
            .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                release.update(cx, |state, cx| {
                    state.pressed = false;
                    cx.notify();
                });
            })
            .on_mouse_up_out(MouseButton::Left, move |_, _, cx| {
                release_out.update(cx, |state, cx| {
                    state.pressed = false;
                    cx.notify();
                });
            })
            .on_hover(move |hovered, _, cx| {
                if !hovered {
                    hover.update(cx, |state, cx| {
                        state.cancel_press();
                        state.pointer = None;
                        cx.notify();
                    });
                }
            })
            .when(interactive, |element| {
                element.on_mouse_move(move |event, _, cx| {
                    pointer.update(cx, |state, cx| {
                        state.pointer = Some(event.position);
                        cx.notify();
                    });
                })
            })
            .children(options);
        root.style().refine(&self.style);
        root.when(self.disabled, |element| {
            element.opacity(appearance.disabled_opacity)
        })
        .focus_visible(move |style| style.border_color(appearance.focus_ring))
    }
}

pub(super) fn next_value<'a, T: PartialEq>(
    values: &'a [T],
    selected: &T,
    key: &str,
) -> Option<&'a T> {
    if values.is_empty() {
        return None;
    }
    let index = values.iter().position(|value| value == selected);
    let index = match key {
        "left" | "up" => index.map_or(values.len() - 1, |i| (i + values.len() - 1) % values.len()),
        "right" | "down" => index.map_or(0, |i| (i + 1) % values.len()),
        "home" => 0,
        "end" => values.len() - 1,
        _ => return None,
    };
    values.get(index)
}
