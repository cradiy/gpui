use gpui::{
    Context, IntoElement, Modifiers, Render, TestAppContext, VisualTestContext, Window, div,
    prelude::*, px, size,
};

use super::GlassSegmentedControl;

struct Demo {
    selected: u8,
    disabled: bool,
    changes: Vec<u8>,
}

impl Render for Demo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        div().p(px(30.)).child(
            GlassSegmentedControl::new("test-glass", self.selected)
                .label("Layout")
                .disabled(self.disabled)
                .animated(false)
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
fn selection_preserves_layout_and_keyboard_skips_disabled_options(cx: &mut TestAppContext) {
    let window = cx.open_window(size(px(650.), px(180.)), |_, _| Demo {
        selected: 0,
        disabled: false,
        changes: Vec::new(),
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
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
    visual.simulate_click(bounds[2].center(), Modifiers::default());
    visual.update(|window, cx| {
        window.draw(cx).clear();
    });
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.selected, 2))
        .unwrap();
    for (selector, bound) in selectors.into_iter().zip(&bounds) {
        assert_eq!(*bound, visual.debug_bounds(selector).unwrap());
    }
    visual.simulate_keystrokes("left");
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.selected, 0))
        .unwrap();
    visual.simulate_keystrokes("left");
    window
        .update(&mut visual.cx, |this, _, _| assert_eq!(this.selected, 3))
        .unwrap();
    visual.simulate_keystrokes("home end");
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.changes, [2, 0, 3, 0, 3])
        })
        .unwrap();
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
    visual.simulate_keystrokes("home");
    window
        .update(&mut visual.cx, |this, _, _| {
            assert_eq!(this.changes, [2, 0, 3, 0, 3])
        })
        .unwrap();
}

#[test]
fn navigation_handles_missing_selection_and_empty_groups() {
    use super::segmented::next_value;
    assert_eq!(next_value(&[1, 3], &2, "right"), Some(&1));
    assert_eq!(next_value(&[1, 3], &2, "left"), Some(&3));
    assert_eq!(next_value(&[] as &[u8], &0, "home"), None);
    assert_eq!(next_value(&[1], &1, "right"), Some(&1));
    assert_eq!(next_value(&[1], &1, "escape"), None);
}
