use std::sync::Arc;

use chrono::{NaiveDate, Weekday};
use gpui::{
    App, ElementId, Entity, FontWeight, IntoElement, RenderOnce, Role, StyleRefinement, Styled,
    Window, div, prelude::*, px, rgb,
};

use super::{CalendarAppearance, CalendarLocale, CalendarState, CalendarView};

/// The localized title for the active calendar projection.
#[derive(IntoElement)]
pub struct CalendarTitle {
    state: Entity<CalendarState>,
    view: Option<CalendarView>,
    locale: CalendarLocale,
    first_weekday: Option<Weekday>,
    style: StyleRefinement,
}

impl CalendarTitle {
    pub fn new(state: &Entity<CalendarState>) -> Self {
        Self {
            state: state.clone(),
            view: None,
            locale: CalendarLocale::default(),
            first_weekday: None,
            style: StyleRefinement::default()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD),
        }
    }

    pub fn locale(mut self, locale: CalendarLocale) -> Self {
        self.locale = locale;
        self
    }

    /// Pins the title to a projection instead of following `CalendarState::view`.
    pub fn view(mut self, view: CalendarView) -> Self {
        self.view = Some(view);
        self
    }

    pub fn first_weekday(mut self, weekday: Weekday) -> Self {
        self.first_weekday = Some(weekday);
        self
    }
}

impl RenderOnce for CalendarTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = self.state.read(cx);
        let weekday = self.first_weekday.unwrap_or(self.locale.first_weekday);
        let view = self.view.unwrap_or(state.view());
        let title = self.locale.title(
            view,
            state.anchor_date(),
            state.visible_range_for(view, weekday),
        );
        let mut root = div().child(title);
        root.style().refine(&self.style);
        root
    }
}

impl Styled for CalendarTitle {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// Previous and next controls for the active projection.
#[derive(IntoElement)]
pub struct CalendarPager {
    id: ElementId,
    state: Entity<CalendarState>,
    locale: CalendarLocale,
    appearance: CalendarAppearance,
    view: Option<CalendarView>,
    first_weekday: Option<Weekday>,
    style: StyleRefinement,
}

impl CalendarPager {
    pub fn new(id: impl Into<ElementId>, state: &Entity<CalendarState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            locale: CalendarLocale::default(),
            appearance: CalendarAppearance::default(),
            view: None,
            first_weekday: None,
            style: StyleRefinement::default().flex().items_center().gap_1(),
        }
    }

    pub fn locale(mut self, locale: CalendarLocale) -> Self {
        self.locale = locale;
        self
    }

    pub fn appearance(mut self, appearance: CalendarAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    /// Pins paging increments to a projection instead of following the state.
    pub fn view(mut self, view: CalendarView) -> Self {
        self.view = Some(view);
        self
    }

    pub fn first_weekday(mut self, weekday: Weekday) -> Self {
        self.first_weekday = Some(weekday);
        self
    }
}

impl RenderOnce for CalendarPager {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let weekday = self.first_weekday.unwrap_or(self.locale.first_weekday);
        let view = self.view;
        let previous_state = self.state.clone();
        let next_state = self.state.clone();
        let previous = div()
            .id((self.id.clone(), "previous"))
            .role(Role::Button)
            .aria_label(self.locale.labels.previous.clone())
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(px(9.))
            .text_lg()
            .text_color(self.appearance.secondary_text)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(self.appearance.hover_day)
                    .text_color(self.appearance.accent)
            })
            .on_click(move |_, _, cx| {
                previous_state.update(cx, |state, cx| {
                    if let Some(view) = view {
                        state.previous_in(view, weekday, cx);
                    } else {
                        state.previous(weekday, cx);
                    }
                });
            })
            .child("‹");
        let next = div()
            .id((self.id.clone(), "next"))
            .role(Role::Button)
            .aria_label(self.locale.labels.next.clone())
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(px(9.))
            .text_lg()
            .text_color(self.appearance.secondary_text)
            .cursor_pointer()
            .hover(|style| {
                style
                    .bg(self.appearance.hover_day)
                    .text_color(self.appearance.accent)
            })
            .on_click(move |_, _, cx| {
                next_state.update(cx, |state, cx| {
                    if let Some(view) = view {
                        state.next_in(view, weekday, cx);
                    } else {
                        state.next(weekday, cx);
                    }
                });
            })
            .child("›");
        let mut root = div().id(self.id).child(previous).child(next);
        root.style().refine(&self.style);
        root
    }
}

impl Styled for CalendarPager {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// A standalone control that moves a calendar to today.
#[derive(IntoElement)]
pub struct CalendarTodayButton {
    id: ElementId,
    state: Entity<CalendarState>,
    locale: CalendarLocale,
    appearance: CalendarAppearance,
    today: NaiveDate,
    style: StyleRefinement,
}

impl CalendarTodayButton {
    pub fn new(id: impl Into<ElementId>, state: &Entity<CalendarState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            locale: CalendarLocale::default(),
            appearance: CalendarAppearance::default(),
            today: chrono::Local::now().date_naive(),
            style: StyleRefinement::default()
                .px(px(14.))
                .h(px(32.))
                .flex()
                .items_center()
                .rounded(px(9.))
                .border_1(),
        }
    }

    pub fn locale(mut self, locale: CalendarLocale) -> Self {
        self.locale = locale;
        self
    }

    pub fn appearance(mut self, appearance: CalendarAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn today(mut self, today: NaiveDate) -> Self {
        self.today = today;
        self
    }
}

impl RenderOnce for CalendarTodayButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let state = self.state.clone();
        let today = self.today;
        let mut root = div()
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.locale.labels.today.clone())
            .border_color(self.appearance.grid_line)
            .bg(rgb(0xffffff))
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(|style| {
                style
                    .border_color(self.appearance.accent)
                    .text_color(self.appearance.accent)
                    .bg(self.appearance.hover_day)
            })
            .on_click(move |_, _, cx| {
                state.update(cx, |state, cx| state.go_to(today, cx));
            })
            .child(self.locale.labels.today.clone());
        root.style().refine(&self.style);
        root
    }
}

impl Styled for CalendarTodayButton {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// A standalone selector for the active calendar projection.
#[derive(IntoElement)]
pub struct CalendarViewSwitcher {
    id: ElementId,
    state: Entity<CalendarState>,
    locale: CalendarLocale,
    appearance: CalendarAppearance,
    views: Arc<[CalendarView]>,
    style: StyleRefinement,
}

impl CalendarViewSwitcher {
    pub fn new(id: impl Into<ElementId>, state: &Entity<CalendarState>) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            locale: CalendarLocale::default(),
            appearance: CalendarAppearance::default(),
            views: Arc::from([
                CalendarView::Year,
                CalendarView::Month,
                CalendarView::Week,
                CalendarView::Day,
            ]),
            style: StyleRefinement::default()
                .flex()
                .items_center()
                .p(px(3.))
                .rounded(px(10.))
                .bg(rgb(0xf1f4f8)),
        }
    }

    pub fn locale(mut self, locale: CalendarLocale) -> Self {
        self.locale = locale;
        self
    }

    pub fn appearance(mut self, appearance: CalendarAppearance) -> Self {
        self.appearance = appearance;
        self
    }

    pub fn views(mut self, views: impl Into<Arc<[CalendarView]>>) -> Self {
        self.views = views.into();
        self
    }
}

impl RenderOnce for CalendarViewSwitcher {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let active_view = self.state.read(cx).view();
        let mut root = div().id(self.id.clone());
        for view in self.views.iter().copied() {
            let state = self.state.clone();
            let active = view == active_view;
            root = root.child(
                div()
                    .id((self.id.clone(), format!("{view:?}")))
                    .role(Role::Button)
                    .aria_selected(active)
                    .px(px(13.))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .cursor_pointer()
                    .text_sm()
                    .text_color(if active {
                        self.appearance.accent
                    } else {
                        self.appearance.secondary_text
                    })
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .when(active, |element| {
                        element
                            .bg(rgb(0xffffff))
                            .border_1()
                            .border_color(rgb(0xe4e9f1))
                            .shadow_sm()
                    })
                    .when(!active, |element| {
                        element.hover(|style| style.text_color(self.appearance.accent))
                    })
                    .on_click(move |_, _, cx| {
                        state.update(cx, |state, cx| state.set_view(view, cx));
                    })
                    .child(self.locale.view_label(view)),
            );
        }
        root.style().refine(&self.style);
        root
    }
}

impl Styled for CalendarViewSwitcher {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
