use std::{cell::Cell, rc::Rc, time::Instant};

use gpui::{
    AnyElement, App, Axis, Bounds, BoxShadow, ElementId, FlexDirection, IntoElement, Pixels,
    RenderOnce, Role, SharedString, StyleRefinement, Styled, Window, canvas, div, fill, hsla,
    point, prelude::*, px, rgb,
};
use gpui_effects::{LiquidGlassRegion, liquid_glass_content, paint_liquid_glass};

use super::{
    GlassSegmentedAppearance,
    interaction::{ChangeCallback, OptionGeometry, State, interaction},
    motion::{PressScales, scale_about_center},
};

struct OptionItem<T> {
    value: T,
    content: AnyElement,
    disabled: bool,
}

/// A controlled, single-selection glass control with an interruptible indicator.
/// Option layout stays fixed while the moving glass refracts its painted content. Give each
/// control a stable ID and update the selected value in `on_change`.
/// Selection and keyboard policy are owned by the caller; this control handles pointers.
#[derive(IntoElement)]
pub struct GlassSegmentedControl<T: Clone + PartialEq + 'static> {
    id: ElementId,
    selected: T,
    options: Vec<OptionItem<T>>,
    on_change: Option<ChangeCallback<T>>,
    label: Option<SharedString>,
    disabled: bool,
    animated: bool,
    press_scales: PressScales,
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
            press_scales: PressScales::default(),
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

    /// Sets the selected lens's held scale (default `1.35`).
    /// Accepts `1.0..=2.0`; `1.0` disables its press scaling. Layout is unchanged.
    #[track_caller]
    pub fn selection_press_scale(mut self, scale: f32) -> Self {
        assert!(
            (1.0..=2.0).contains(&scale),
            "selection press scale must be in 1.0..=2.0"
        );
        self.press_scales.selection = scale;
        self
    }

    /// Sets the outer glass's held scale (default `1.05`).
    /// Accepts `1.0..=2.0`; `1.0` disables its press scaling. Layout is unchanged.
    #[track_caller]
    pub fn surface_press_scale(mut self, scale: f32) -> Self {
        assert!(
            (1.0..=2.0).contains(&scale),
            "surface press scale must be in 1.0..=2.0"
        );
        self.press_scales.surface = scale;
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
        let motion = window.use_keyed_state(self.id.clone(), cx, |_, _| State::new());
        let geometries = Rc::new(
            self.options
                .iter()
                .map(|o| OptionGeometry {
                    value: o.value.clone(),
                    disabled: o.disabled,
                    bounds: Cell::new(None),
                })
                .collect::<Vec<_>>(),
        );
        let root_bounds = Rc::new(Cell::new(None));
        let pointer_interaction = interaction(
            motion.clone(),
            geometries.clone(),
            self.selected.clone(),
            root_bounds.clone(),
            self.on_change.clone(),
            !self.disabled,
        );
        let selected_bounds = Rc::new(Cell::new(None::<Bounds<Pixels>>));
        let content_region = Rc::new(Cell::new(None::<LiquidGlassRegion>));
        let appearance = self.appearance;
        let options = self
            .options
            .into_iter()
            .enumerate()
            .map(|(index, option)| {
                let selected = option.value == self.selected;
                let disabled = self.disabled || option.disabled;
                let target = selected_bounds.clone();
                let geometry = geometries.clone();
                let region = content_region.clone();
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
                    .when(selected, |element| element.aria_active_descendant())
                    .when(disabled && !self.disabled, |element| {
                        element.opacity(appearance.disabled_opacity)
                    })
                    .when(!disabled, |element| element.cursor_pointer())
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                geometry[index].bounds.set(Some(bounds));
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
                    .child(liquid_glass_content(
                        option.content,
                        appearance.selected_text,
                        move |_, _| region.get(),
                    ))
            })
            .collect::<Vec<_>>();
        let reduced = self.reduced_transparency || !window.supports_backdrop_blur();
        let animated = self.animated;
        let press_scales = self.press_scales;
        let disabled = self.disabled;
        let lens = Rc::new(Cell::new(None));
        let lens_to_paint = lens.clone();
        let painted = div().on_paint_before_children(move |bounds, style, window, cx| {
            lens.set(None);
            root_bounds.set(Some(bounds));
            content_region.set(None);
            let corners = style
                .corner_radii
                .to_pixels(window.rem_size())
                .clamp_radii_for_quad_size(bounds.size);
            let mut fallback = appearance.surface.tint;
            fallback.a = 1.;
            let (visual, pointer) = motion.update(cx, |state, _| {
                state.motion.press_scales = press_scales;
                let axis = match style.flex_direction {
                    FlexDirection::Column | FlexDirection::ColumnReverse => Axis::Vertical,
                    FlexDirection::Row | FlexDirection::RowReverse => Axis::Horizontal,
                };
                if state.axis != axis {
                    state.cancel(window);
                    state.axis = axis;
                }
                if disabled || !window.is_window_active() {
                    state.cancel(window);
                }
                let target = selected_bounds
                    .get()
                    .filter(|selected| {
                        selected.size.width > px(0.) && selected.size.height > px(0.)
                    })
                    .map(|selected| Bounds::new(selected.origin - bounds.origin, selected.size));
                (
                    state.motion.sample(target, Instant::now(), animated),
                    state.motion.pointer,
                )
            });
            let surface_scale = visual.as_ref().map_or(1., |visual| visual.surface_scale);
            let surface_bounds = scale_about_center(bounds, surface_scale);
            let surface_corners = corners
                .map(|radius| *radius * surface_scale)
                .clamp_radii_for_quad_size(surface_bounds.size);
            if reduced {
                window.paint_quad(fill(surface_bounds, fallback).corner_radii(surface_corners));
            } else {
                let mut surface = appearance.surface;
                surface.thickness *= surface_scale;
                surface.refraction *= surface_scale;
                paint_liquid_glass(surface_bounds, surface_corners, surface, window);
            }
            let Some(visual) = visual else {
                return;
            };
            if visual.moving && window.is_window_active() {
                window.request_animation_frame();
            }
            let mut selected = visual.bounds;
            selected.origin += bounds.origin;
            let selected_corners = corners
                .map(|radius| (*radius - px(4.)).max(px(0.)) * visual.scale)
                .clamp_radii_for_quad_size(selected.size);
            content_region.set(Some(LiquidGlassRegion {
                bounds: if reduced {
                    Bounds::from_corners(
                        window.pixel_snap_point(selected.origin),
                        window.pixel_snap_point(selected.bottom_right()),
                    )
                } else {
                    selected
                },
                corner_radii: selected_corners,
                deformation: Default::default(),
            }));
            window.paint_drop_shadows(
                selected,
                selected_corners,
                &[BoxShadow::new(
                    px(0.),
                    px(2.5 + visual.pressure * 3.5),
                    hsla(0.6, 0.2, 0.05, 0.1 + visual.pressure * 0.06),
                )
                .blur_radius(px(7. + visual.pressure * 9.))],
            );
            let mut optics = appearance.selection;
            optics.highlight *= 1. + visual.pressure * 0.12;
            optics.thickness = (optics.thickness.max(px(0.)) * visual.scale)
                .min(selected.size.width.min(selected.size.height) * 0.1);
            // Smoothstep's maximum slope is 1.5 / thickness. Keep every color
            // channel below that fold threshold so edge sampling cannot repeat
            // interior strokes. The flat center retains zero displacement.
            let dispersion = optics.dispersion.clamp(0., 0.1);
            optics.refraction = (optics.refraction.max(px(0.)) * visual.scale)
                .min(optics.thickness * 0.5 / (1. + dispersion));
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
                lens.set(Some((selected, selected_corners, optics)));
            }
        });
        let mut root = painted
            .id(self.id)
            .relative()
            .debug_selector(|| "uic-glass-segmented".into())
            .role(Role::RadioGroup)
            .when_some(self.label, |element, label| element.aria_label(label))
            .children(options)
            .child(
                canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        if let Some((bounds, corners, optics)) = lens_to_paint.get() {
                            paint_liquid_glass(bounds, corners, optics, window);
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .child(pointer_interaction);
        root.style().refine(&self.style);
        root.when(self.disabled, |element| {
            element.opacity(appearance.disabled_opacity)
        })
    }
}
