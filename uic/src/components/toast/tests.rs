use super::*;
use gpui::{TestAppContext, VisualTestContext, size};

#[test]
fn toast_appearance_uses_styled_for_its_surface() {
    let appearance = ToastAppearance::default().w(px(360.)).opacity(0.9);
    assert!(appearance.style.size.width.is_some());
    assert_eq!(appearance.style.opacity, Some(0.9));
}

#[test]
fn variants_have_distinct_status_colors() {
    let colors = ToastAppearance::default().colors;
    assert_ne!(colors.success, colors.error);
    assert_ne!(colors.info, colors.warn);
}

#[gpui::test]
fn messages_wrap_inside_the_surface_and_follow_window_width(cx: &mut TestAppContext) {
    let messages = [
        "Saved successfully".to_owned(),
        "The selected files could not be saved. Please choose another directory and try again. "
            .repeat(3),
        format!(
            "/home/user/Documents/{}.svg",
            "VeryLongFileNameWithoutSpaces".repeat(8)
        ),
        "导出失败，请检查目标文件夹是否存在以及是否有写入权限。".repeat(6),
    ];
    for placement in [ToastPlacement::Top, ToastPlacement::Bottom] {
        let handle = cx.open_window(size(px(900.), px(1200.)), |_, _| ToastManager {
            items: vec![ToastItem {
                id: 1,
                message: messages[0].clone().into(),
                variant: ToastVariant::Info,
                placement,
            }],
            ..ToastManager::new(ToastAppearance::default())
        });
        let mut visual = VisualTestContext::from_window(handle.into(), cx);
        for message in &messages {
            handle
                .update(&mut visual.cx, |manager, _, cx| {
                    manager.items[0].message = message.clone().into();
                    cx.notify();
                })
                .unwrap();
            let mut wide_height = px(0.);
            for width in [900., 240., 900.] {
                visual.simulate_resize(size(px(width), px(1200.)));
                visual.update(|window, cx| window.draw(cx).clear());
                let surface = visual.debug_bounds("toast-1").unwrap();
                let text = visual.debug_bounds("toast-message-1").unwrap();
                let icon = visual.debug_bounds("toast-icon-1").unwrap();
                assert!(
                    surface.left() >= px(24.) && surface.right() <= px(width - 24.),
                    "{surface:?}"
                );
                assert!(surface.size.width <= px(480.));
                assert!(text.left() > icon.right());
                assert!(text.right() <= surface.right() - px(12.));
                assert!(text.bottom() <= surface.bottom() - px(9.));
                assert_eq!(text.top(), icon.top());
                assert_eq!(icon.size.height, px(22.));
                if message != &messages[0] {
                    assert!(
                        text.size.height > px(22.),
                        "long messages must wrap: {text:?}"
                    );
                }
                if width == 240. && message != &messages[0] {
                    assert!(
                        text.size.height > wide_height,
                        "narrower toast must grow vertically"
                    );
                } else if width == 900. {
                    if wide_height > px(0.) {
                        assert_eq!(text.size.height, wide_height);
                    }
                    wide_height = text.size.height;
                }
                if placement == ToastPlacement::Top {
                    assert_eq!(surface.top(), px(24.));
                } else {
                    assert_eq!(surface.bottom(), px(1176.));
                }
            }
        }
    }
}

#[gpui::test]
fn styled_width_is_bounded_by_the_viewport(cx: &mut TestAppContext) {
    let handle = cx.open_window(size(px(320.), px(800.)), |_, _| ToastManager {
        items: vec![ToastItem {
            id: 1,
            message: "long/path/".repeat(30).into(),
            variant: ToastVariant::Success,
            placement: ToastPlacement::Top,
        }],
        ..ToastManager::new(
            ToastAppearance::default()
                .w(px(800.))
                .max_w(px(1000.))
                .viewport_margin(px(16.)),
        )
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.update(|window, cx| window.draw(cx).clear());
    let surface = visual.debug_bounds("toast-1").unwrap();
    assert_eq!(surface.size.width, px(288.));
    assert_eq!(surface.left(), px(16.));
    handle
        .update(&mut visual.cx, |manager, _, cx| {
            manager.appearance = ToastAppearance::default().max_w(px(180.));
            cx.notify();
        })
        .unwrap();
    visual.update(|window, cx| window.draw(cx).clear());
    assert_eq!(visual.debug_bounds("toast-1").unwrap().size.width, px(180.));
}
