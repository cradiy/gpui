use gpui::{
    AppContext, Context, Entity, InspectorElement, Modifiers, Render, StyleRefinement,
    TestAppContext, VisualTestContext, Window, div, point, prelude::*, px, size,
};

struct Content {
    width: f32,
    clicks: usize,
    visible: bool,
}
impl Render for Content {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().id("content").size_full().p_4().child(
            div()
                .id("parent")
                .w(px(250.))
                .h(px(150.))
                .p_2()
                .overflow_hidden()
                .when(self.visible, |parent| {
                    parent.child(
                        div()
                            .id("target")
                            .w(px(self.width))
                            .h(px(60.))
                            .p_2()
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.clicks += 1;
                                cx.notify();
                            }))
                            .child("Target"),
                    )
                }),
        )
    }
}
struct Root(Entity<Content>);
impl Render for Root {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.0.clone().cached(StyleRefinement {
            size: gpui::SizeRefinement {
                width: Some(gpui::relative(1.).into()),
                height: Some(gpui::relative(1.).into()),
            },
            ..Default::default()
        })
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<Content>, VisualTestContext) {
    cx.update(gpui_inspector::init);
    let content = cx.new(|_| Content {
        width: 100.,
        clicks: 0,
        visible: true,
    });
    let handle = cx.open_window(size(px(1000.), px(700.)), |_, _| Root(content.clone()));
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|window, cx| window.toggle_inspector(cx));
    draw(&mut visual);
    (content, visual)
}
fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear());
}
fn snapshot(cx: &mut VisualTestContext) -> Vec<InspectorElement> {
    cx.update(|window, _| window.inspector_elements().to_vec())
}
fn target(elements: &[InspectorElement]) -> &InspectorElement {
    elements
        .iter()
        .find(|node| node.id.path.global_id.to_string().ends_with("target") && node.style.is_some())
        .unwrap()
}

#[gpui::test]
fn picking_preserves_application_state_and_selected_layout_stays_live(cx: &mut TestAppContext) {
    let (content, mut cx) = setup(cx);
    let nodes = snapshot(&mut cx);
    let node = target(&nodes);
    assert_eq!(node.bounds.size.width, px(100.));
    let parent = &nodes[node.parent.unwrap()];
    assert!(parent.id.path.global_id.to_string().ends_with("parent"));
    assert!(node.content_mask.bounds.size.width <= px(250.));
    let click = node.bounds.origin + point(px(3.), px(3.));
    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_click(click, Modifiers::default());
    draw(&mut cx);
    cx.update(|window, cx| {
        let inspector = window.inspector().unwrap();
        assert!(!inspector.read(cx).is_picking());
        assert_eq!(inspector.read(cx).active_element_id(), Some(&node.id));
        assert_eq!(content.read(cx).clicks, 0);
        content.update(cx, |content, cx| {
            content.width = 180.;
            cx.notify();
        });
    });
    draw(&mut cx);
    assert_eq!(target(&snapshot(&mut cx)).bounds.size.width, px(180.));
    // A frame that would otherwise reuse a cached view still has the full tree.
    draw(&mut cx);
    assert_eq!(target(&snapshot(&mut cx)).bounds.size.width, px(180.));
    cx.simulate_click(click, Modifiers::default());
    cx.update(|_, cx| assert_eq!(content.read(cx).clicks, 1));
    cx.update(|_, cx| {
        content.update(cx, |content, cx| {
            content.visible = false;
            cx.notify();
        })
    });
    draw(&mut cx);
    assert!(
        !snapshot(&mut cx)
            .iter()
            .any(|node| node.id == target(&nodes).id)
    );
}

#[gpui::test]
fn panel_controls_work_while_picking_and_tree_can_collapse(cx: &mut TestAppContext) {
    let (_, mut cx) = setup(cx);
    // Closing must work even though picking intercepts clicks in the application.
    let close = cx.debug_bounds("inspector-close").unwrap().center();
    cx.simulate_click(close, Modifiers::default());
    draw(&mut cx);
    cx.update(|window, _| assert!(!window.is_inspector_open()));
    assert!(snapshot(&mut cx).is_empty());
    cx.update(|window, cx| window.toggle_inspector(cx));
    draw(&mut cx);
    assert!(cx.debug_bounds("inspector-row-1").is_some());
    let collapse = cx.debug_bounds("inspector-expand-0").unwrap().center();
    cx.simulate_click(collapse, Modifiers::default());
    draw(&mut cx);
    assert!(cx.debug_bounds("inspector-row-1").is_none());
    cx.simulate_click(collapse, Modifiers::default());
    draw(&mut cx);
    let row = cx.debug_bounds("inspector-row-1").unwrap().center();
    let expected = snapshot(&mut cx)[1].id.clone();
    cx.simulate_click(row, Modifiers::default());
    draw(&mut cx);
    cx.update(|window, cx| {
        assert_eq!(
            window.inspector().unwrap().read(cx).active_element_id(),
            Some(&expected)
        )
    });
}

#[gpui::test]
fn explicit_style_edits_remain_overrides(cx: &mut TestAppContext) {
    let (content, mut cx) = setup(cx);
    let id = target(&snapshot(&mut cx)).id.clone();
    cx.update(|window, cx| {
        window
            .inspector()
            .unwrap()
            .update(cx, |inspector, _| inspector.select(id.clone(), window));
    });
    draw(&mut cx);
    cx.update(|window, cx| {
        window.with_inspector_state(
            Some(&id),
            cx,
            |state: &mut Option<gpui::DivInspectorState>, _| {
                state.as_mut().unwrap().base_style.size.width = Some(px(140.).into());
            },
        );
        content.update(cx, |content, cx| {
            content.width = 200.;
            cx.notify();
        });
    });
    draw(&mut cx);
    assert_eq!(target(&snapshot(&mut cx)).bounds.size.width, px(140.));
    draw(&mut cx);
    assert_eq!(target(&snapshot(&mut cx)).bounds.size.width, px(140.));
    cx.update(|window, cx| window.toggle_inspector(cx));
    draw(&mut cx);
    cx.update(|window, cx| window.toggle_inspector(cx));
    draw(&mut cx);
    assert_eq!(target(&snapshot(&mut cx)).bounds.size.width, px(200.));
}

#[gpui::test]
fn deferred_content_and_custom_inspector_details_are_prepainted_once(cx: &mut TestAppContext) {
    struct DeferredContent;
    impl Render for DeferredContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(gpui::deferred(
                div().id("app-overlay").size(px(80.)).child("Overlay"),
            ))
        }
    }
    cx.update(|cx| {
        gpui_inspector::init(cx);
        cx.register_inspector_element::<gpui::DivInspectorState, _>(|_, _, _, _| {
            gpui::deferred(
                div()
                    .id("custom-inspector-detail")
                    .debug_selector(|| "custom-inspector-detail".to_owned())
                    .h(px(20.))
                    .child("Custom detail"),
            )
        });
    });
    let handle = cx.open_window(size(px(1000.), px(800.)), |window, cx| {
        window.toggle_inspector(cx);
        DeferredContent
    });
    let mut cx = VisualTestContext::from_window(handle.into(), cx);
    draw(&mut cx);
    let nodes = snapshot(&mut cx);
    let overlay = nodes
        .iter()
        .find(|node| node.id.path.global_id.to_string().ends_with("app-overlay"))
        .unwrap();
    assert!(overlay.parent.is_none());
    cx.update(|window, cx| {
        window.inspector().unwrap().update(cx, |inspector, _| {
            inspector.select(overlay.id.clone(), window)
        })
    });
    draw(&mut cx);
    assert!(cx.debug_bounds("custom-inspector-detail").is_some());
    assert_eq!(snapshot(&mut cx).len(), nodes.len());
    draw(&mut cx);
    assert!(cx.debug_bounds("custom-inspector-detail").is_some());
}

#[gpui::test]
fn resolved_spacing_inherited_text_and_report_match_layout(cx: &mut TestAppContext) {
    struct StyledContent;
    impl Render for StyledContent {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .id("styled-root")
                .size_full()
                .text_size(px(18.))
                .text_color(gpui::rgb(0x336699))
                .child(
                    div().w(px(300.)).h(px(200.)).flex().flex_col().child(
                        div()
                            .id("target")
                            .w(px(120.))
                            .h(px(80.))
                            .flex_shrink_0()
                            .mx_auto()
                            .p(gpui::relative(0.1))
                            .border_2()
                            .child("中文".repeat(150)),
                    ),
                )
        }
    }
    cx.update(gpui_inspector::init);
    let handle = cx.open_window(size(px(1000.), px(800.)), |window, cx| {
        window.toggle_inspector(cx);
        StyledContent
    });
    let mut cx = VisualTestContext::from_window(handle.into(), cx);
    draw(&mut cx);
    let nodes = snapshot(&mut cx);
    let node = target(&nodes);
    assert_eq!(node.box_model.margin.left, px(90.));
    assert_eq!(node.box_model.margin.right, px(90.));
    assert_eq!(node.box_model.padding.top, px(30.));
    assert_eq!(node.box_model.padding.left, px(30.));
    assert_eq!(node.box_model.border.left, px(2.));
    assert_eq!(node.text_style.color, gpui::rgb(0x336699).into());
    assert_eq!(node.text_style.font_size, px(18.).into());
    assert_eq!(node.text.len(), 1);
    assert_eq!(node.text[0].preview.chars().count(), 256);
    assert!(node.text[0].preview_shortened);
    assert_eq!(node.text[0].base_style.color, node.text_style.color);
    cx.update(|window, cx| {
        window
            .inspector()
            .unwrap()
            .update(cx, |inspector, _| inspector.select(node.id.clone(), window))
    });
    draw(&mut cx);
    let text_tab = cx.debug_bounds("inspector-text").unwrap().center();
    cx.simulate_click(text_tab, Modifiers::default());
    draw(&mut cx);
    let copy = cx.debug_bounds("inspector-copy").unwrap().center();
    cx.simulate_click(copy, Modifiers::default());
    cx.run_until_parked();
    let report = cx.update(|_, cx| cx.read_from_clipboard().unwrap().text().unwrap());
    assert!(report.contains("Content box: 56.0 × 16.0 px"), "{report}");
    assert!(report.contains("Text color source: Inherited"));
    assert!(report.contains("#336699FF"));
    assert!(report.contains("Padding T R B L: 10% · 10% · 10% · 10%"));
    assert!(report.contains("Resolved margin T R B L: 0.0px · 90.0px · 0.0px · 90.0px"));
    assert!(report.contains("中文"));
    let toggle = cx.debug_bounds("inspector-highlight").unwrap().center();
    cx.simulate_click(toggle, Modifiers::default());
    cx.update(|window, cx| {
        let inspector = window.inspector().unwrap();
        assert!(!inspector.read(cx).is_highlighting());
        assert_eq!(inspector.read(cx).active_element_id(), Some(&node.id));
    });
}
