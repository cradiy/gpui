use crate::{
    AppContext, Bounds, Context, ElementBounds, PointerTransform, Render, TestAppContext,
    TransformationMatrix, Window, canvas, point, prelude::*, px, size,
};

struct Probe {
    tracked: ElementBounds,
    discarded: ElementBounds,
    visible: bool,
}

fn source() -> Bounds<crate::Pixels> {
    Bounds::new(point(px(10.), px(20.)), size(px(40.), px(20.)))
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let tracked = self.tracked.clone();
        let discarded = self.discarded.clone();
        let visible = self.visible;
        canvas(
            move |bounds, window, _| {
                assert!(
                    tracked.bounds(window).is_none(),
                    "previous-frame geometry must not leak into prepaint"
                );
                if !visible {
                    return;
                }
                let transform = PointerTransform::affine(TransformationMatrix {
                    rotation_scale: [[1., -1.], [1., 1.]],
                    translation: [200., 50.],
                })
                .unwrap();
                window.with_pointer_transform(bounds, transform, |window| {
                    window.track_element_bounds(&tracked, source());
                    let before = tracked.bounds(window);
                    let _: Result<(), ()> = window.transact(|window| {
                        window.track_element_bounds(&tracked, Bounds::default());
                        window.track_element_bounds(&discarded, source());
                        assert!(discarded.bounds(window).is_some());
                        Err(())
                    });
                    assert_eq!(tracked.bounds(window), before);
                    assert!(discarded.bounds(window).is_none());
                });
            },
            |_, _, _, _| {},
        )
        .size_full()
    }
}

#[crate::test]
fn element_bounds_tracks_current_frame_rollbacks_and_removal(cx: &mut TestAppContext) {
    let tracked = ElementBounds::default();
    let discarded = ElementBounds::default();
    let handle = cx.add_window({
        let (tracked, discarded) = (tracked.clone(), discarded.clone());
        move |_, _| Probe {
            tracked,
            discarded,
            visible: true,
        }
    });
    cx.update_window(handle.into(), |_, window, cx| {
        for _ in 0..3 {
            window.draw(cx).clear();
            let bounds = tracked.bounds(window).unwrap();
            assert_eq!(
                bounds,
                Bounds::new(point(px(170.), px(80.)), size(px(60.), px(60.)))
            );
            assert!(tracked.contains(bounds.center(), window));
            assert!(!tracked.contains(point(px(171.), px(81.)), window));
            assert!(discarded.bounds(window).is_none());
        }
    })
    .unwrap();
    let other = cx.add_window(|_, _| crate::EmptyView);
    cx.update_window(other.into(), |_, window, cx| {
        window.draw(cx).clear();
        assert!(tracked.bounds(window).is_none());
    })
    .unwrap();
    handle
        .update(cx, |view, _, cx| {
            view.visible = false;
            cx.notify();
        })
        .unwrap();
    cx.update_window(handle.into(), |_, window, cx| {
        window.draw(cx).clear();
        assert!(tracked.bounds(window).is_none());
        assert!(!tracked.contains(source().center(), window));
    })
    .unwrap();
}
