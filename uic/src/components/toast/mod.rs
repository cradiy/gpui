use std::{f32::consts::TAU, time::Duration};

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, Context, Entity, Global, Hsla, Length, Pixels,
    Refineable as _, Render, SharedString, StyleRefinement, Styled, Transformation, Window, div,
    prelude::*, px, radians, rgb, svg,
};

use crate::assets::LucideIcons;

const DEFAULT_DURATION: Duration = Duration::from_secs(3);

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastVariant {
    #[default]
    Info,
    Success,
    Warn,
    Error,
    Loading,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToastPlacement {
    #[default]
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, uic_macros::Chainable)]
pub struct ToastColors {
    pub info: Hsla,
    pub success: Hsla,
    pub warn: Hsla,
    pub error: Hsla,
    pub loading: Hsla,
}

impl ToastColors {
    fn color(self, variant: ToastVariant) -> Hsla {
        match variant {
            ToastVariant::Info => self.info,
            ToastVariant::Success => self.success,
            ToastVariant::Warn => self.warn,
            ToastVariant::Error => self.error,
            ToastVariant::Loading => self.loading,
        }
    }
}

#[derive(Clone, Debug, uic_macros::Chainable)]
pub struct ToastAppearance {
    pub colors: ToastColors,
    pub gap: Pixels,
    #[chain(skip)]
    style: StyleRefinement,
    #[chain(skip)]
    viewport_margin: Pixels,
}

impl ToastAppearance {
    pub fn viewport_margin(mut self, margin: Pixels) -> Self {
        self.viewport_margin = margin.max(px(0.));
        self
    }
}

impl Styled for ToastAppearance {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl Default for ToastAppearance {
    fn default() -> Self {
        Self {
            colors: ToastColors {
                info: rgb(0x1677ff).into(),
                success: rgb(0x52c41a).into(),
                warn: rgb(0xfaad14).into(),
                error: rgb(0xff4d4f).into(),
                loading: rgb(0x1677ff).into(),
            },
            gap: px(10.),
            style: StyleRefinement::default()
                .max_w(px(480.))
                .px(px(12.))
                .py(px(9.))
                .rounded(px(8.))
                .border_1()
                .border_color(rgb(0x000000).opacity(0.06))
                .bg(rgb(0xffffff))
                .text_color(rgb(0x000000).opacity(0.88))
                .shadow_lg(),
            viewport_margin: px(24.),
        }
    }
}

#[derive(Clone)]
struct ToastItem {
    id: u64,
    message: SharedString,
    variant: ToastVariant,
    placement: ToastPlacement,
}

pub struct ToastManager {
    items: Vec<ToastItem>,
    next_id: u64,
    appearance: ToastAppearance,
    placement: ToastPlacement,
}

impl ToastManager {
    fn new(appearance: ToastAppearance) -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
            appearance,
            placement: ToastPlacement::default(),
        }
    }

    fn push(
        &mut self,
        message: impl Into<SharedString>,
        variant: ToastVariant,
        duration: Duration,
        placement: Option<ToastPlacement>,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(ToastItem {
            id,
            message: message.into(),
            variant,
            placement: placement.unwrap_or(self.placement),
        });
        cx.notify();

        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(duration).await;
            if let Some(this) = this.upgrade() {
                this.update(cx, |manager, cx| manager.dismiss(id, cx));
            }
        })
        .detach();
    }

    fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.items.retain(|item| item.id != id);
        cx.notify();
    }

    fn render_stack(&self, placement: ToastPlacement, window: &Window) -> gpui::Div {
        let appearance = &self.appearance;
        let available_width =
            (window.viewport_size().width - appearance.viewport_margin * 2.).max(px(0.));
        let max_width = match appearance.style.max_size.width {
            Some(Length::Definite(width)) => width
                .to_pixels(available_width.into(), window.rem_size())
                .min(available_width),
            _ => available_width,
        };
        div()
            .absolute()
            .left(appearance.viewport_margin)
            .right(appearance.viewport_margin)
            .when_else(
                placement == ToastPlacement::Top,
                |this| this.top(appearance.viewport_margin).flex_col(),
                |this| this.bottom(appearance.viewport_margin).flex_col_reverse(),
            )
            .flex()
            .items_center()
            .gap(appearance.gap)
            .children(
                self.items
                    .iter()
                    .filter(move |item| item.placement == placement)
                    .map(|item| {
                        let mut toast = div()
                            .id(("toast", item.id as usize))
                            .debug_selector(|| format!("toast-{}", item.id))
                            .flex()
                            .flex_shrink_0()
                            .items_start()
                            .gap_2()
                            .child(
                                div()
                                    .debug_selector(|| format!("toast-icon-{}", item.id))
                                    .flex_none()
                                    .h(px(22.))
                                    .flex()
                                    .items_center()
                                    .child(status_icon(
                                        item.variant,
                                        appearance.colors.color(item.variant),
                                        item.id,
                                    )),
                            )
                            .child(
                                div()
                                    .debug_selector(|| format!("toast-message-{}", item.id))
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_sm()
                                    .line_height(px(22.))
                                    .child(item.message.clone()),
                            );
                        toast.style().refine(&appearance.style);
                        toast.min_w_0().max_w(max_width)
                    }),
            )
    }
}

impl Render for ToastManager {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().absolute().inset_0().children([
            self.render_stack(ToastPlacement::Top, window),
            self.render_stack(ToastPlacement::Bottom, window),
        ])
    }
}

fn status_icon(variant: ToastVariant, color: Hsla, id: u64) -> AnyElement {
    let icon = match variant {
        ToastVariant::Info => LucideIcons::Info,
        ToastVariant::Success => LucideIcons::CircleCheck,
        ToastVariant::Warn => LucideIcons::CircleAlert,
        ToastVariant::Error => LucideIcons::CircleX,
        ToastVariant::Loading => LucideIcons::Loader,
    };
    let icon = svg().path(icon).size_4().flex_none().text_color(color);

    if variant == ToastVariant::Loading {
        icon.with_animation(
            ("toast-loading-icon", id as usize),
            Animation::new(Duration::from_millis(900)).repeat(),
            |icon, phase| icon.with_transformation(Transformation::rotate(radians(phase * TAU))),
        )
        .into_any_element()
    } else {
        icon.into_any_element()
    }
}

struct GlobalToast(Entity<ToastManager>);

impl Global for GlobalToast {}

pub fn init(cx: &mut App) {
    init_with_appearance(ToastAppearance::default(), cx);
}

pub fn init_with_appearance(appearance: ToastAppearance, cx: &mut App) {
    if !cx.has_global::<GlobalToast>() {
        let manager = cx.new(|_| ToastManager::new(appearance));
        cx.set_global(GlobalToast(manager));
    } else {
        set_appearance(appearance, cx);
    }
}

pub fn set_appearance(appearance: ToastAppearance, cx: &mut App) {
    layer(cx).update(cx, |manager, cx| {
        manager.appearance = appearance;
        cx.notify();
    });
}

pub fn set_placement(placement: ToastPlacement, cx: &mut App) {
    layer(cx).update(cx, |manager, cx| {
        manager.placement = placement;
        cx.notify();
    });
}

pub fn layer(cx: &App) -> Entity<ToastManager> {
    cx.global::<GlobalToast>().0.clone()
}

pub fn info(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastVariant::Info, DEFAULT_DURATION, cx);
}

pub fn success(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastVariant::Success, DEFAULT_DURATION, cx);
}

pub fn warn(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastVariant::Warn, DEFAULT_DURATION, cx);
}

pub fn error(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastVariant::Error, DEFAULT_DURATION, cx);
}

pub fn loading(message: impl Into<SharedString>, cx: &mut App) {
    show(message, ToastVariant::Loading, DEFAULT_DURATION, cx);
}

pub fn success_at(message: impl Into<SharedString>, placement: ToastPlacement, cx: &mut App) {
    show_at(
        message,
        ToastVariant::Success,
        DEFAULT_DURATION,
        placement,
        cx,
    );
}

pub fn warn_at(message: impl Into<SharedString>, placement: ToastPlacement, cx: &mut App) {
    show_at(message, ToastVariant::Warn, DEFAULT_DURATION, placement, cx);
}

pub fn error_at(message: impl Into<SharedString>, placement: ToastPlacement, cx: &mut App) {
    show_at(
        message,
        ToastVariant::Error,
        DEFAULT_DURATION,
        placement,
        cx,
    );
}

pub fn show(
    message: impl Into<SharedString>,
    variant: ToastVariant,
    duration: Duration,
    cx: &mut App,
) {
    layer(cx).update(cx, |manager, cx| {
        manager.push(message, variant, duration, None, cx);
    });
}

pub fn show_at(
    message: impl Into<SharedString>,
    variant: ToastVariant,
    duration: Duration,
    placement: ToastPlacement,
    cx: &mut App,
) {
    layer(cx).update(cx, |manager, cx| {
        manager.push(message, variant, duration, Some(placement), cx);
    });
}
