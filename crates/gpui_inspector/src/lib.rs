#![doc = include_str!("../README.md")]

use gpui::{
    AnyElement, App, Context, Div, Inspector, InspectorElementId, IntoElement, Stateful, Window,
    div, prelude::*, px, rgb,
};
use std::collections::HashSet;

mod details;
mod interaction;
mod theme;

/// Registers the inspector UI. The application controls window toggles and keyboard bindings.
pub fn init(cx: &mut App) {
    cx.set_inspector_renderer(Box::new(render));
}

fn button(id: &'static str, label: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(|| id.to_owned())
        .px_2()
        .py_1()
        .text_size(px(11.))
        .rounded(px(5.))
        .cursor_pointer()
        .text_color(rgb(theme::MUTED))
        .hover(|style| style.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT)))
        .child(label)
}

fn property(label: &str, value: impl Into<String>) -> Div {
    div()
        .flex()
        .gap_2()
        .py(px(5.))
        .child(
            div()
                .w(px(106.))
                .flex_shrink_0()
                .text_color(rgb(theme::MUTED))
                .child(label.to_owned()),
        )
        .child(div().flex_1().min_w_0().child(value.into()))
}

fn render(
    inspector: &mut Inspector,
    window: &mut Window,
    cx: &mut Context<Inspector>,
) -> AnyElement {
    let collapsed = window.use_keyed_state("inspector-collapsed", cx, |_, _| {
        HashSet::<InspectorElementId>::new()
    });
    let active_tab = window.use_keyed_state("inspector-tab", cx, |_, _| 0usize);
    let previous_selection = window.use_keyed_state("inspector-last-selection", cx, |_, _| {
        None::<InspectorElementId>
    });
    let copy_status = window.use_keyed_state("inspector-copy-status", cx, |_, _| "");
    let tree_height = px((f32::from(window.viewport_size().height) * 0.24).clamp(80., 220.));
    let previous_tree_height = window.use_keyed_state("inspector-tree-height", cx, |_, _| px(0.));
    let tree_resized = *previous_tree_height.read(cx) != tree_height;
    if tree_resized {
        previous_tree_height.update(cx, |height, _| *height = tree_height);
    }
    let tree_scroll = window
        .use_keyed_state("inspector-tree-scroll", cx, |_, _| {
            gpui::ScrollHandle::new()
        })
        .read(cx)
        .clone();
    let elements = window.inspector_elements();
    // Prune state for elements no longer present in this window.
    collapsed.update(cx, |collapsed, _| {
        let present: HashSet<_> = elements.iter().map(|node| &node.id).collect();
        collapsed.retain(|id| present.contains(id));
    });
    let selected = inspector.active_element_id();
    let selection_changed = previous_selection.read(cx).as_ref() != selected;
    if selection_changed {
        previous_selection.update(cx, |previous, _| *previous = selected.cloned());
        copy_status.update(cx, |status, _| *status = "");
        if let Some(index) = elements
            .iter()
            .position(|element| Some(&element.id) == selected)
        {
            collapsed.update(cx, |collapsed, _| {
                let mut parent = elements[index].parent;
                while let Some(index) = parent {
                    collapsed.remove(&elements[index].id);
                    parent = elements[index].parent;
                }
            });
        }
    }
    let selected_element = elements
        .iter()
        .find(|element| Some(&element.id) == selected)
        .cloned();
    let mut hidden = vec![false; elements.len()];
    let mut depths = vec![0; elements.len()];
    let mut parents = vec![false; elements.len()];
    for element in elements {
        if let Some(parent) = element.parent {
            parents[parent] = true;
        }
    }
    let mut rows = Vec::new();
    let mut reveal_row = None;
    for (index, element) in elements.iter().enumerate() {
        if let Some(parent) = element.parent {
            depths[index] = depths[parent] + 1;
            hidden[index] = hidden[parent] || collapsed.read(cx).contains(&elements[parent].id);
        }
        if hidden[index] {
            continue;
        }
        let id = element.id.clone();
        let is_selected = Some(&id) == selected;
        if is_selected && (selection_changed || tree_resized) {
            reveal_row = Some(rows.len());
        }
        let is_collapsed = collapsed.read(cx).contains(&id);
        let type_name = element
            .type_name
            .split('<')
            .next()
            .unwrap_or(element.type_name)
            .rsplit("::")
            .next()
            .unwrap_or(element.type_name);
        let type_name = if type_name == "Stateful" {
            element
                .type_name
                .split_once('<')
                .map(|(_, inner)| {
                    inner
                        .trim_end_matches('>')
                        .split('<')
                        .next()
                        .unwrap_or(inner)
                        .rsplit("::")
                        .next()
                        .unwrap_or(inner)
                })
                .unwrap_or(type_name)
        } else {
            type_name
        };
        let name = if element
            .parent
            .is_none_or(|parent| elements[parent].id.path.global_id != element.id.path.global_id)
        {
            match element.id.path.global_id.last() {
                Some(id) => format!("{id} · {type_name}"),
                None => type_name.to_owned(),
            }
        } else {
            type_name.to_owned()
        };
        let name = if let Some(text) = element.text.first() {
            let preview = text
                .preview
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            format!(
                "{name} ‘{}{}’",
                preview.chars().take(28).collect::<String>(),
                if preview.chars().count() > 28 {
                    "…"
                } else {
                    ""
                }
            )
        } else {
            name
        };
        let has_children = parents[index];
        let marker = if has_children {
            if is_collapsed { "▸" } else { "▾" }
        } else {
            "·"
        };
        let toggle_id = id.clone();
        let collapsed = collapsed.clone();
        rows.push(
            div()
                .id(("element", index))
                .debug_selector(move || format!("inspector-row-{index}"))
                .flex()
                .items_center()
                .h(px(27.))
                .flex_shrink_0()
                .pl(px(8. + depths[index] as f32 * 12.))
                .pr_2()
                .gap_1()
                .cursor_pointer()
                .border_l_2()
                .border_color(gpui::transparent_black())
                .when(is_selected, |row| {
                    row.bg(rgb(theme::SELECTED))
                        .border_color(rgb(theme::ACCENT))
                })
                .hover(|style| style.bg(rgb(theme::HOVER)))
                .on_click(cx.listener(move |inspector, _, window, cx| {
                    inspector.select(id.clone(), window);
                    cx.notify();
                }))
                .child(
                    div()
                        .id(("expand", index))
                        .debug_selector(move || format!("inspector-expand-{index}"))
                        .w(px(20.))
                        .flex_shrink_0()
                        .child(marker)
                        .on_click(move |_, window, cx| {
                            if has_children {
                                collapsed.update(cx, |collapsed, cx| {
                                    if !collapsed.remove(&toggle_id) {
                                        collapsed.insert(toggle_id.clone());
                                    }
                                    cx.notify();
                                });
                                window.refresh();
                                cx.stop_propagation();
                            }
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(name),
                )
                .child(
                    div()
                        .text_color(rgb(theme::MUTED))
                        .text_size(px(10.))
                        .flex_shrink_0()
                        .child(format!(
                            "{:.0}×{:.0}",
                            f32::from(element.bounds.size.width),
                            f32::from(element.bounds.size.height)
                        )),
                ),
        );
    }
    let count = elements.len();
    let picking = inspector.is_picking();
    let mut details = div()
        .id("inspector-properties")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .p_3();
    let mut navigation = div()
        .flex()
        .flex_col()
        .gap_1()
        .px_3()
        .py_2()
        .flex_shrink_0();
    if let Some(element) = selected_element {
        let parent = element.parent.map(|index| elements[index].clone());
        let mut actions = div().flex().flex_wrap().items_center().gap_2();
        if let Some(parent) = &parent {
            let parent_id = parent.id.clone();
            actions = actions.child(button("inspector-parent", "Parent").on_click(cx.listener(
                move |inspector, _, window, cx| {
                    inspector.select(parent_id.clone(), window);
                    cx.notify();
                },
            )));
        }
        let report_element = element.clone();
        let report_parent = parent.clone();
        let probe = inspector
            .pointer_position()
            .unwrap_or(element.bounds.center());
        let hits = window.inspector_hitboxes_at(probe);
        let hit_report = interaction::report(probe, &hits);
        let status = *copy_status.read(cx);
        actions = actions
            .child(
                button("inspector-copy", "Copy report").on_click(cx.listener(
                    move |_, _, _, cx| {
                        let task =
                            cx.write_to_clipboard_async(gpui::ClipboardItem::new_string(format!(
                                "{}\n{}",
                                details::report(&report_element, report_parent.as_ref()),
                                hit_report
                            )));
                        let status = copy_status.clone();
                        cx.spawn(async move |_, cx| {
                            let result = task.await;
                            let _ = status.update(cx, |status, cx| {
                                *status = if result.is_ok() {
                                    "Copied"
                                } else {
                                    "Copy failed"
                                };
                                cx.notify();
                            });
                        })
                        .detach();
                    },
                )),
            )
            .when(!status.is_empty(), |actions| {
                actions.child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(theme::ACCENT))
                        .child(status),
                )
            });
        actions = actions.child(
            button(
                "inspector-highlight",
                if inspector.is_highlighting() {
                    "Hide overlay"
                } else {
                    "Show overlay"
                },
            )
            .on_click(cx.listener(|inspector, _, window, cx| {
                inspector.set_highlighting(!inspector.is_highlighting(), window);
                cx.notify();
            })),
        );
        navigation = navigation
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .mb_1()
                    .child(div().size(px(6.)).rounded_full().bg(rgb(theme::ACCENT)))
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(
                                element
                                    .id
                                    .path
                                    .global_id
                                    .last()
                                    .map(|id| id.to_string())
                                    .unwrap_or_else(|| "Element".into()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(rgb(theme::MUTED))
                            .child(format!(
                                "{:.0} × {:.0}",
                                f32::from(element.bounds.size.width),
                                f32::from(element.bounds.size.height)
                            )),
                    ),
            )
            .child(actions);
        let tab = *active_tab.read(cx);
        navigation = navigation.child(
            div()
                .flex()
                .mt_1()
                .p(px(3.))
                .rounded(px(7.))
                .bg(rgb(theme::BACKGROUND))
                .children(
                    [
                        (0, "inspector-layout", "Layout"),
                        (1, "inspector-style", "Style"),
                        (2, "inspector-text", "Text"),
                        (4, "inspector-interaction", "Input"),
                        (3, "inspector-source", "Source"),
                    ]
                    .into_iter()
                    .map(|(index, id, label)| {
                        let state = active_tab.clone();
                        button(id, label)
                            .flex_1()
                            .min_w_0()
                            .text_center()
                            .when(tab == index, |button| {
                                button.bg(rgb(theme::HOVER)).text_color(rgb(theme::TEXT))
                            })
                            .on_click(move |_, window, cx| {
                                state.update(cx, |tab, cx| {
                                    *tab = index;
                                    cx.notify();
                                });
                                window.refresh();
                            })
                    }),
                ),
        );
        if tab == 0 {
            details = details.child(details::box_model(&element));
        }
        if tab == 4 {
            details = details.child(interaction::render(
                probe,
                &hits,
                inspector.is_picking(),
                cx,
            ));
        }
        details = details.child(details::render(details::sections(
            &element,
            parent.as_ref(),
            tab,
        )));
    } else {
        details = details.child(
            div()
                .text_color(rgb(0x94a3b8))
                .child(if selected.is_some() {
                    "Selected element is no longer in the rendered tree."
                } else {
                    "Pick an element in the application, or select a row above."
                }),
        );
    }
    details = details.children(inspector.render_inspector_states(window, cx));
    if let Some(index) = reveal_row {
        let scroll = tree_scroll.clone();
        window.on_next_frame(move |window, _| {
            scroll.scroll_to_item(index);
            window.refresh();
        });
    }
    div()
        .id("gpui-inspector")
        .size_full()
        .flex()
        .flex_col()
        .overflow_hidden()
        .bg(rgb(theme::BACKGROUND))
        .text_color(rgb(theme::TEXT))
        .text_size(px(12.))
        .border_l_1()
        .border_color(rgb(theme::BORDER))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px_3()
                .py_2()
                .flex_shrink_0()
                .child(
                    div()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_size(px(14.))
                        .child("Inspector"),
                )
                .child(
                    button("inspector-close", "Close")
                        .on_click(|_, window, cx| window.toggle_inspector(cx)),
                ),
        )
        .child(
            div()
                .px_3()
                .pb_2()
                .flex()
                .items_center()
                .gap_3()
                .flex_shrink_0()
                .child(
                    button(
                        "inspector-pick",
                        if picking {
                            "Stop picking"
                        } else {
                            "Pick element"
                        },
                    )
                    .bg(rgb(theme::SELECTED))
                    .text_color(rgb(theme::ACCENT))
                    .on_click(cx.listener(|inspector, _, window, cx| {
                        if inspector.is_picking() {
                            inspector.stop_picking();
                        } else {
                            inspector.start_picking();
                        }
                        window.refresh();
                        cx.notify();
                    })),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(rgb(theme::MUTED))
                        .child(format!("{count} elements")),
                ),
        )
        .child(
            div()
                .px_3()
                .py_1()
                .border_t_1()
                .border_color(rgb(theme::BORDER))
                .text_size(px(10.))
                .text_color(rgb(theme::MUTED))
                .child("ELEMENT TREE"),
        )
        .child(
            div()
                .id("inspector-tree")
                .h(tree_height)
                .flex_shrink_0()
                .overflow_y_scroll()
                .track_scroll(&tree_scroll)
                .bg(rgb(theme::SURFACE))
                .children(rows),
        )
        .child(div().h(px(1.)).flex_shrink_0().bg(rgb(theme::BORDER)))
        .child(navigation)
        .child(details)
        .child(
            div()
                .px_3()
                .py_2()
                .flex_shrink_0()
                .border_t_1()
                .border_color(rgb(theme::BORDER))
                .text_size(px(10.))
                .text_color(rgb(theme::MUTED))
                .child(if picking {
                    "Click to select · Scroll to pick an ancestor"
                } else {
                    "Logical pixels · Live inspection"
                }),
        )
        .into_any_element()
}

fn edges<T: Clone + std::fmt::Debug + Default + PartialEq>(
    edges: gpui::Edges<T>,
    format: impl Fn(T) -> String,
) -> String {
    [edges.top, edges.right, edges.bottom, edges.left]
        .map(format)
        .join(" · ")
}

fn definite(length: gpui::DefiniteLength) -> String {
    match length {
        gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Pixels(value)) => {
            format!("{:.1}px", f32::from(value))
        }
        gpui::DefiniteLength::Absolute(gpui::AbsoluteLength::Rems(value)) => format!("{value:?}"),
        gpui::DefiniteLength::Fraction(value) => format!("{:.0}%", value * 100.),
    }
}
