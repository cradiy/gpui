use crate::{
    AnyTooltip, AppContext, Context, EmptyView, Entity, FocusHandle, Keystroke, Render,
    TestAppContext, Window, canvas, div, point, prelude::*, px, size,
};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};

crate::actions!(prepaint_retry, [Activate]);

struct Control {
    focus: FocusHandle,
    keys: Rc<Cell<usize>>,
    actions: Rc<Cell<usize>>,
    modifiers: Rc<Cell<usize>>,
    renders: Rc<Cell<usize>>,
    tooltip: AnyTooltip,
}

impl Render for Control {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.renders.set(self.renders.get() + 1);
        let keys = self.keys.clone();
        let actions = self.actions.clone();
        let modifiers = self.modifiers.clone();
        let tooltip = self.tooltip.clone();
        div()
            .id("control")
            .size_full()
            .track_focus(&self.focus)
            .on_key_down(move |_, _, _| keys.set(keys.get() + 1))
            .on_action(move |_: &Activate, _, _| actions.set(actions.get() + 1))
            .on_modifiers_changed(move |_, _, _| modifiers.set(modifiers.get() + 1))
            .child(canvas(
                move |_, window, _| {
                    window.set_tooltip(tooltip);
                },
                |_, _, _, _| {},
            ))
    }
}

struct Board {
    control: Entity<Control>,
    retry: Rc<Cell<u8>>,
    reuse_ranges: bool,
    range: Rc<RefCell<Option<Range<crate::window::PrepaintStateIndex>>>>,
}

fn prepaint_with_retry<T>(
    window: &mut Window,
    mode: u8,
    mut prepaint: impl FnMut(&mut Window) -> T,
) -> T {
    match mode {
        1 => {
            let _: Result<(), ()> = window.transact(|window| {
                prepaint(window);
                Err(())
            });
        }
        2 => {
            let _: Result<(), ()> = window.transact(|window| {
                window
                    .transact(|window| {
                        prepaint(window);
                        Ok::<_, ()>(())
                    })
                    .unwrap();
                Err(())
            });
        }
        3 => {
            return window
                .transact(|window| {
                    let _: Result<(), ()> = window.transact(|window| {
                        prepaint(window);
                        Err(())
                    });
                    Ok::<_, ()>(prepaint(window))
                })
                .unwrap();
        }
        _ => {}
    }
    prepaint(window)
}

impl Render for Board {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let control = self.control.clone();
        let retry = self.retry.clone();
        let ranges = self.range.clone();
        let reuse_ranges = self.reuse_ranges;
        canvas(
            move |_, window, cx| {
                if reuse_ranges && let Some(range) = ranges.borrow().clone() {
                    return prepaint_with_retry(window, retry.get(), |window| {
                        let start = window.prepaint_index();
                        window.reuse_prepaint(range.clone());
                        let end = window.prepaint_index();
                        (None::<crate::AnyElement>, start..end)
                    });
                }
                let child = || {
                    control
                        .clone()
                        .cached(div().size_full().style().clone())
                        .into_any_element()
                };
                prepaint_with_retry(window, retry.get(), |window| {
                    let start = window.prepaint_index();
                    let mut child = child();
                    child.prepaint_as_root(
                        point(px(0.), px(0.)),
                        size(px(100.), px(40.)).into(),
                        window,
                        cx,
                    );
                    let end = window.prepaint_index();
                    (Some(child), start..end)
                })
            },
            {
                let range = self.range.clone();
                move |_,
                      (child, next_range): (
                    Option<crate::AnyElement>,
                    Range<crate::window::PrepaintStateIndex>,
                ),
                      window,
                      cx| {
                    if let Some(mut child) = child {
                        child.paint(window, cx);
                    }
                    *range.borrow_mut() = Some(next_range);
                }
            },
        )
        .size_full()
    }
}

fn check_retry(cx: &mut TestAppContext, reuse_ranges: bool) {
    let keys = Rc::new(Cell::new(0));
    let actions = Rc::new(Cell::new(0));
    let modifiers = Rc::new(Cell::new(0));
    let renders = Rc::new(Cell::new(0));
    let retry = Rc::new(Cell::new(0));
    let handle = cx.add_window({
        let (keys, renders, retry) = (keys.clone(), renders.clone(), retry.clone());
        let (actions, modifiers) = (actions.clone(), modifiers.clone());
        move |window, cx| {
            let tooltip = AnyTooltip {
                view: cx.new(|_| EmptyView).into(),
                mouse_position: point(px(10.), px(10.)),
                check_visible_and_update: Rc::new(|_, _, _| true),
            };
            let focus = cx.focus_handle();
            focus.focus(window, cx);
            Board {
                control: cx.new(|_| Control {
                    focus,
                    keys,
                    actions,
                    modifiers,
                    renders,
                    tooltip,
                }),
                retry,
                reuse_ranges,
                range: Default::default(),
            }
        }
    });
    for (attempt, mode) in [1, 0, 1, 2, 3, 1, 0, 0].into_iter().enumerate() {
        retry.set(mode);
        let renders_before = renders.get();
        cx.update_window(handle.into(), |_, window, cx| {
            window.draw(cx).clear();
            window.dispatch_keystroke(Keystroke::parse("a").unwrap(), cx);
            assert_eq!(
                keys.get(),
                attempt + 1,
                "cached key handler must survive a failed prepaint"
            );
            window
                .focused(cx)
                .unwrap()
                .dispatch_action(&Activate, window, cx);
            assert_eq!(actions.get(), attempt + 1);
            window.dispatch_event(
                crate::PlatformInput::ModifiersChanged(crate::ModifiersChangedEvent::default()),
                cx,
            );
            assert_eq!(modifiers.get(), attempt + 1);
            assert_eq!(window.rendered_frame.tooltip_requests.len(), 1);
            assert!(
                window.rendered_frame.tooltip_requests[0].is_some(),
                "cached tooltip must survive a failed prepaint"
            );
        })
        .unwrap();
        if attempt > 0 && (mode == 0 || reuse_ranges) {
            assert_eq!(
                renders.get(),
                renders_before,
                "successful draws must still reuse cached content"
            );
        }
    }
}

#[crate::test]
fn reused_prepaint_retry_preserves_keyboard_and_tooltip(cx: &mut TestAppContext) {
    check_retry(cx, true);
}

#[crate::test]
fn cached_view_prepaint_retry_preserves_keyboard_and_tooltip(cx: &mut TestAppContext) {
    check_retry(cx, false);
}
