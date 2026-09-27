use gpui::{
    Context, IntoElement, Modifiers, MouseButton, Render, TestAppContext, VisualTestContext,
    Window, div, point, prelude::*, px, size,
};

use super::GlassSegmentedControl;

struct Demo {
    selected: u8,
    vertical: bool,
    disabled: bool,
    changes: Vec<u8>,
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div().p(px(30.)).child(
            GlassSegmentedControl::new("test-glass", self.selected)
                .label("Layout")
                .when(self.vertical, |control| {
                    control.flex_col().items_stretch().w(px(180.))
                })
                .disabled(self.disabled)
                .animated(false)
                .selection_press_scale(1.5)
                .surface_press_scale(1.08)
                .text_size(px(18.))
                .p(px(7.))
                .rounded(px(17.))
                .option(0, "Day")
                .disabled_option(1, "Unavailable")
                .option(2, "Working week")
                .option(3, "Month")
                .on_change(move |value, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.selected = value;
                        this.changes.push(value);
                        cx.notify();
                    });
                }),
        )
    }
}

#[gpui::test]
fn pointer_selection_preserves_layout_and_skips_disabled_options(cx: &mut TestAppContext) {
    let window = cx.open_window(size(px(650.), px(180.)), |_, _| Demo {
        selected: 0,
        vertical: false,
        disabled: false,
        changes: Vec::new(),
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear();
    });
    let selectors = [
        "uic-glass-segment-0",
        "uic-glass-segment-1",
        "uic-glass-segment-2",
        "uic-glass-segment-3",
    ];
    let bounds: Vec<_> = selectors
        .into_iter()
        .map(|selector| visual.debug_bounds(selector).unwrap())
        .collect();
    assert!(bounds[2].size.width > bounds[0].size.width);
    let root = visual.debug_bounds("uic-glass-segmented").unwrap();
    assert!(bounds[0].origin.x - root.origin.x >= px(7.));
    visual.simulate_click(bounds[1].center(), Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert!(this.changes.is_empty())
        })
        .unwrap();
    visual.simulate_mouse_down(bounds[2].center(), MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.selected, 2);
            assert_eq!(this.changes, [2]);
        })
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    assert_eq!(root, visual.debug_bounds("uic-glass-segmented").unwrap());
    for (selector, bound) in selectors.into_iter().zip(&bounds) {
        assert_eq!(*bound, visual.debug_bounds(selector).unwrap());
    }
    visual.simulate_mouse_up(bounds[2].center(), MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.changes, [2]))
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.selected, 2))
        .unwrap();
    for (selector, bound) in selectors.into_iter().zip(&bounds) {
        assert_eq!(*bound, visual.debug_bounds(selector).unwrap());
    }
    window
        .update(&mut visual.cx, |this, _, cx| {
            this.disabled = true;
            cx.notify();
        })
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    visual.simulate_click(bounds[0].center(), Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.changes, [2]))
        .unwrap();
}

#[gpui::test]
fn dragging_captures_pointer_commits_once_and_can_cancel(cx: &mut TestAppContext) {
    let window = cx.open_window(size(px(650.), px(180.)), |_, _| Demo {
        selected: 0,
        vertical: false,
        disabled: false,
        changes: Vec::new(),
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear();
    });
    visual.background_executor.run_until_parked();
    let first = visual.debug_bounds("uic-glass-segment-0").unwrap().center();
    let disabled = visual.debug_bounds("uic-glass-segment-1").unwrap().center();
    let last = visual.debug_bounds("uic-glass-segment-3").unwrap().center();
    let outside = point(px(645.), px(150.));
    visual.simulate_mouse_down(first, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(disabled, MouseButton::Left, Modifiers::default());
    visual.update(|window, cx| {
        window.draw(cx).clear();
        assert!(window.captured_hitbox().is_some());
    });
    window
        .update(&mut visual.cx, |this, _, _| {
            assert!(this.changes.is_empty())
        })
        .unwrap();
    visual.simulate_mouse_move(outside, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.selected, 3);
            assert_eq!(this.changes, [3]);
        })
        .unwrap();
    visual.update(|window, _| assert!(window.captured_hitbox().is_none()));

    // A press on another option changes selection before dragging begins.
    // The drag offset must start at that option, not at the previous lens.
    visual.simulate_mouse_down(first, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.selected, 0);
            assert_eq!(this.changes, [3, 0]);
        })
        .unwrap();
    visual.simulate_mouse_move(last, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(last, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.selected, 3);
            assert_eq!(this.changes, [3, 0, 3]);
        })
        .unwrap();

    visual.simulate_mouse_down(last, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(first, MouseButton::Left, Modifiers::default());
    visual.deactivate_window();
    visual.simulate_mouse_up(first, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.changes, [3, 0, 3])
        })
        .unwrap();
    visual.update(|window, _| window.activate_window());
    visual.background_executor.run_until_parked();

    visual.simulate_mouse_down(last, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(first, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, cx| {
            this.disabled = true;
            cx.notify();
        })
        .unwrap();
    visual.simulate_mouse_up(first, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.changes, [3, 0, 3])
        })
        .unwrap();
}

#[gpui::test]
fn vertical_layout_selects_on_press_drags_on_y_and_cancels_on_axis_change(cx: &mut TestAppContext) {
    let window = cx.open_window(size(px(650.), px(360.)), |_, _| Demo {
        selected: 0,
        vertical: true,
        disabled: false,
        changes: Vec::new(),
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear();
    });
    visual.background_executor.run_until_parked();
    let first = visual.debug_bounds("uic-glass-segment-0").unwrap();
    let last = visual.debug_bounds("uic-glass-segment-3").unwrap();
    assert_eq!(first.center().x, last.center().x);
    assert!(first.bottom() < last.top());
    visual.simulate_mouse_down(last.center(), MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.changes, [3]))
        .unwrap();
    let outside = point(first.center().x + px(200.), px(5.));
    visual.simulate_mouse_move(outside, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.changes, [3, 0])
        })
        .unwrap();
    visual.simulate_mouse_down(first.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(last.center(), MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, cx| {
            this.vertical = false;
            cx.notify();
        })
        .unwrap();
    visual.update(|window, cx| {
        window.draw(cx).clear();
        assert!(window.captured_hitbox().is_none());
    });
    visual.simulate_mouse_up(last.center(), MouseButton::Left, Modifiers::default());
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.changes, [3, 0])
        })
        .unwrap();
}

#[gpui::test]
fn pointer_selection_preserves_host_focus_and_keyboard_events(cx: &mut TestAppContext) {
    struct Host {
        focus: gpui::FocusHandle,
        selected: u8,
        keys: Vec<String>,
    }
    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let entity = cx.entity();
            div()
                .id("host")
                .track_focus(&self.focus)
                .p(px(30.))
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                    this.keys.push(event.keystroke.key.clone());
                    cx.notify();
                }))
                .child(
                    GlassSegmentedControl::new("default-glass", self.selected)
                        .animated(false)
                        .option(0, "First")
                        .option(1, "Second")
                        .on_change(move |value, _, cx| {
                            entity.update(cx, |this, cx| {
                                this.selected = value;
                                cx.notify();
                            });
                        }),
                )
        }
    }
    let window = cx.open_window(size(px(450.), px(180.)), |window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Host {
            focus,
            selected: 0,
            keys: Vec::new(),
        }
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.activate_window();
        window.draw(cx).clear();
    });
    visual.background_executor.run_until_parked();
    let second = visual.debug_bounds("uic-glass-segment-1").unwrap();
    visual.simulate_click(second.center(), Modifiers::default());
    window
        .update(&mut visual.cx, |this, window, _| {
            assert_eq!(this.selected, 1);
            assert!(this.focus.is_focused(window));
        })
        .unwrap();
    visual.simulate_keystrokes("left home end escape");
    window
        .update(&mut visual.cx, |this, window, _| {
            assert_eq!(this.selected, 1);
            assert!(this.focus.is_focused(window));
            assert_eq!(this.keys, ["left", "home", "end", "escape"]);
        })
        .unwrap();
}
