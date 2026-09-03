use std::rc::Rc;

use gpui::{
    AccessibleAction, AnyElement, App, ElementId, Entity, IntoElement, RenderOnce, Role,
    SharedString, StyleRefinement, Styled, Window, div, prelude::*, px, rgb, svg,
    transparent_black,
};

use crate::assets::LucideIcons;

use super::{CollapsibleAppearance, CollapsibleState};

type IndicatorRenderer = Rc<dyn Fn(bool) -> AnyElement>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CollapsibleIndicatorPosition {
    #[default]
    Start,
    End,
}

/// One independently composable disclosure panel.
///
/// Multiple items share a [`CollapsibleState`] when they should behave as a
/// coordinated multi-open group or single-open accordion.
#[derive(IntoElement)]
pub struct Collapsible {
    id: ElementId,
    state: Entity<CollapsibleState>,
    item: SharedString,
    header: Option<AnyElement>,
    content: Option<AnyElement>,
    accessible_label: Option<SharedString>,
    disabled: bool,
    indicator_position: CollapsibleIndicatorPosition,
    indicator_renderer: Option<IndicatorRenderer>,
    appearance: CollapsibleAppearance,
    style: StyleRefinement,
}

impl Collapsible {
    pub fn new(
        id: impl Into<ElementId>,
        state: &Entity<CollapsibleState>,
        item: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            state: state.clone(),
            item: item.into(),
            header: None,
            content: None,
            accessible_label: None,
            disabled: false,
            indicator_position: CollapsibleIndicatorPosition::Start,
            indicator_renderer: None,
            appearance: CollapsibleAppearance::default(),
            style: StyleRefinement::default()
                .w_full()
                .rounded(px(14.))
                .border_1()
                .border_color(rgb(0xe4e9f1))
                .bg(rgb(0xffffff))
                .text_color(rgb(0x172033))
                .overflow_hidden(),
        }
    }

    pub fn header(mut self, header: impl IntoElement) -> Self {
        self.header = Some(header.into_any_element());
        self
    }

    pub fn header_text(mut self, header: impl Into<SharedString>) -> Self {
        let header = header.into();
        self.accessible_label = Some(header.clone());
        self.header = Some(
            div()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(header)
                .into_any_element(),
        );
        self
    }

    pub fn content(mut self, content: impl IntoElement) -> Self {
        self.content = Some(content.into_any_element());
        self
    }

    pub fn aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn indicator_position(mut self, position: CollapsibleIndicatorPosition) -> Self {
        self.indicator_position = position;
        self
    }

    pub fn indicator<E>(mut self, renderer: impl Fn(bool) -> E + 'static) -> Self
    where
        E: IntoElement,
    {
        self.indicator_renderer = Some(Rc::new(move |expanded| {
            renderer(expanded).into_any_element()
        }));
        self
    }

    pub fn appearance(mut self, appearance: CollapsibleAppearance) -> Self {
        self.appearance = appearance;
        self
    }
}

impl RenderOnce for Collapsible {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let expanded = self.state.read(cx).is_expanded(self.item.as_ref());
        let indicator = self
            .indicator_renderer
            .map(|renderer| renderer(expanded))
            .unwrap_or_else(|| {
                svg()
                    .path(if expanded {
                        LucideIcons::ChevronDown
                    } else {
                        LucideIcons::ChevronRight
                    })
                    .size_4()
                    .text_color(self.appearance.indicator)
                    .into_any_element()
            });
        let state_for_click = self.state.clone();
        let state_for_key = self.state.clone();
        let state_for_action = self.state.clone();
        let item_for_click = self.item.clone();
        let item_for_key = self.item.clone();
        let item_for_action = self.item;
        let header_content = div().min_w_0().flex_1().children(self.header);

        let header = div()
            .id((self.id.clone(), "header"))
            .debug_selector(|| "uic-collapsible-header".to_string())
            .focusable()
            .tab_stop(!self.disabled)
            .role(Role::Button)
            .aria_expanded(expanded)
            .when_some(self.accessible_label, |header, label| {
                header.aria_label(label)
            })
            .h(px(52.))
            .px_4()
            .flex()
            .items_center()
            .gap_3()
            .border_2()
            .border_color(transparent_black())
            .opacity(if self.disabled {
                self.appearance.disabled_opacity
            } else {
                1.0
            });

        let mut header = match self.indicator_position {
            CollapsibleIndicatorPosition::Start => header.child(indicator).child(header_content),
            CollapsibleIndicatorPosition::End => header.child(header_content).child(indicator),
        };

        if !self.disabled {
            header = header
                .cursor_pointer()
                .hover(|style| style.bg(self.appearance.hover_background))
                .on_click(move |_, _, cx| {
                    state_for_click
                        .update(cx, |state, cx| state.toggle(item_for_click.clone(), cx));
                })
                .on_key_down(move |event, _, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        state_for_key
                            .update(cx, |state, cx| state.toggle(item_for_key.clone(), cx));
                        cx.stop_propagation();
                    }
                })
                .on_a11y_action(AccessibleAction::Click, move |_, _, cx| {
                    state_for_action
                        .update(cx, |state, cx| state.toggle(item_for_action.clone(), cx));
                });
        }

        header = header.focus_visible(|style| style.border_color(self.appearance.focus_ring));

        let mut root = div()
            .id(self.id)
            .debug_selector(|| "uic-collapsible".to_string())
            .child(header)
            .when(expanded, |root| {
                root.child(
                    div()
                        .border_t_1()
                        .border_color(self.appearance.divider)
                        .p_4()
                        .children(self.content),
                )
            });
        root.style().refine(&self.style);
        root
    }
}

impl Styled for Collapsible {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
