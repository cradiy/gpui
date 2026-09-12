use super::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetActiveWindow;

#[test]
fn unconstrained_placement_honors_anchor_gravity_and_offset() {
    assert_eq!(
        axis(100., 20., 80., 3., -1, 1, 0., 500., false, false, false),
        (103., 80.)
    );
    assert_eq!(
        axis(100., 20., 80., 3., 1, -1, 0., 500., false, false, false),
        (43., 80.)
    );
    assert_eq!(
        axis(100., 20., 80., 3., 0, 0, 0., 500., false, false, false),
        (73., 80.)
    );
    assert_eq!(
        axis(-120., 20., 80., 0., -1, 1, -500., 0., false, false, false),
        (-120., 80.)
    );
}

#[test]
fn constraints_flip_before_sliding_and_only_resize_when_allowed() {
    assert_eq!(
        axis(90., 10., 40., 0., 1, 1, 0., 120., true, true, true),
        (50., 40.)
    );
    assert_eq!(
        axis(90., 10., 40., 0., 1, 1, 0., 120., false, true, false),
        (80., 40.)
    );
    assert_eq!(
        axis(90., 10., 40., 0., 1, 1, 0., 120., false, false, true),
        (100., 20.)
    );
    assert_eq!(
        axis(90., 10., 40., 0., 1, 1, 0., 120., false, false, false),
        (100., 40.)
    );
    assert_eq!(
        axis(40., 10., 200., 0., 1, 1, 0., 120., true, true, true),
        (0., 120.)
    );
}

#[test]
fn all_anchor_and_gravity_directions_are_distinct() {
    let anchors = [
        PopupAnchor::Center,
        PopupAnchor::Top,
        PopupAnchor::Bottom,
        PopupAnchor::Left,
        PopupAnchor::Right,
        PopupAnchor::TopLeft,
        PopupAnchor::TopRight,
        PopupAnchor::BottomLeft,
        PopupAnchor::BottomRight,
    ];
    let gravities = [
        PopupGravity::Center,
        PopupGravity::Top,
        PopupGravity::Bottom,
        PopupGravity::Left,
        PopupGravity::Right,
        PopupGravity::TopLeft,
        PopupGravity::TopRight,
        PopupGravity::BottomLeft,
        PopupGravity::BottomRight,
    ];
    for (anchor, gravity) in anchors.into_iter().zip(gravities) {
        assert_eq!(anchor_axes(anchor), gravity_axes(gravity));
    }
    assert_eq!(
        anchors
            .into_iter()
            .map(anchor_axes)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        9
    );
}

fn handle(id: u64) -> gpui::AnyWindowHandle {
    gpui::WindowHandle::<gpui::Empty>::new(id.into()).into()
}

fn params(kind: gpui::WindowKind) -> gpui::WindowParams {
    gpui::WindowParams {
        bounds: gpui::bounds(
            gpui::point(gpui::px(100.), gpui::px(100.)),
            gpui::size(gpui::px(240.), gpui::px(180.)),
        ),
        titlebar: None,
        kind,
        is_movable: true,
        app_owns_titlebar_drag: false,
        is_resizable: true,
        is_minimizable: true,
        focus: false,
        show: false,
        icon: None,
        display_id: None,
        app_id: None,
        window_min_size: None,
    }
}

fn options(parent: gpui::AnyWindowHandle) -> PopupOptions {
    PopupOptions {
        parent,
        anchor_rect: gpui::bounds(
            gpui::point(gpui::px(20.), gpui::px(30.)),
            gpui::size(gpui::px(40.), gpui::px(20.)),
        ),
        anchor: PopupAnchor::BottomLeft,
        gravity: PopupGravity::BottomRight,
        offset: gpui::point(gpui::px(3.), gpui::px(4.)),
        constraint_adjustment: Adjustment::empty(),
        grab: false,
    }
}

#[test]
#[ignore = "requires a Windows desktop and GPU"]
fn native_popup_preserves_owner_focus_placement_and_lifetime() -> Result<()> {
    use gpui::Platform;
    let platform = crate::WindowsPlatform::new(false)?;
    let parent = platform.open_window(handle(1), params(gpui::WindowKind::Normal))?;
    let parent_hwnd = parent.get_raw_handle();
    let active = unsafe { GetActiveWindow() };
    let popup = platform.open_window(
        handle(2),
        params(gpui::WindowKind::AnchoredPopup(options(handle(1)))),
    )?;
    let hwnd = popup.get_raw_handle();
    let inner = crate::window_from_hwnd(hwnd).unwrap();
    assert_eq!(unsafe { GetWindow(hwnd, GW_OWNER)? }, parent_hwnd);
    assert!(!inner.is_movable && !inner.is_resizable && !inner.is_minimizable);
    assert_eq!(
        inner
            .handle_msg(
                hwnd,
                WM_MOUSEACTIVATE,
                Default::default(),
                Default::default()
            )
            .0,
        MA_NOACTIVATE as isize
    );
    assert_eq!(unsafe { GetActiveWindow() }, active);
    assert_eq!(
        popup.content_size(),
        gpui::size(gpui::px(240.), gpui::px(180.))
    );
    let mut actual = RECT::default();
    unsafe {
        GetWindowRect(hwnd, &mut actual)?;
    }
    let expected = inner.popup.as_ref().unwrap().bounds()?;
    assert_eq!(actual, expected);
    unsafe {
        SetWindowPos(
            parent_hwnd,
            None,
            320,
            260,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        )?;
    }
    unsafe {
        GetWindowRect(hwnd, &mut actual)?;
    }
    assert_eq!(actual, inner.popup.as_ref().unwrap().bounds()?);
    assert_ne!(actual, expected);
    let nested = platform.open_window(
        handle(3),
        params(gpui::WindowKind::AnchoredPopup(options(handle(2)))),
    )?;
    assert_eq!(
        unsafe { GetWindow(nested.get_raw_handle(), GW_OWNER)? },
        hwnd
    );
    unsafe {
        DestroyWindow(parent_hwnd)?;
    }
    assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    assert!(!unsafe { IsWindow(Some(nested.get_raw_handle())) }.as_bool());
    Ok(())
}

#[test]
#[ignore = "requires a Windows desktop and GPU"]
fn unknown_popup_parent_is_rejected() -> Result<()> {
    use gpui::Platform;
    let platform = crate::WindowsPlatform::new(false)?;
    assert!(
        platform
            .open_window(
                handle(2),
                params(gpui::WindowKind::AnchoredPopup(options(handle(99))))
            )
            .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires a Windows desktop and GPU"]
fn grabbing_popups_require_input_and_close_in_nested_order() -> Result<()> {
    use gpui::Platform;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetKeyboardState, SetKeyboardState, VK_ESCAPE, VK_LBUTTON,
    };
    struct KeyboardState([u8; 256]);
    impl Drop for KeyboardState {
        fn drop(&mut self) {
            unsafe { SetKeyboardState(&self.0).unwrap() };
        }
    }
    let mut previous = [0; 256];
    unsafe {
        GetKeyboardState(&mut previous)?;
    }
    let _restore = KeyboardState(previous);
    unsafe {
        SetKeyboardState(&[0; 256])?;
    }
    let platform = crate::WindowsPlatform::new(false)?;
    let parent = platform.open_window(handle(1), params(gpui::WindowKind::Normal))?;
    let mut menu_options = options(handle(1));
    menu_options.grab = true;
    assert!(
        platform
            .open_window(
                handle(2),
                params(gpui::WindowKind::AnchoredPopup(menu_options.clone()))
            )
            .is_err()
    );
    let mut pressed = [0; 256];
    pressed[VK_LBUTTON.0 as usize] = 0x80;
    unsafe {
        SetKeyboardState(&pressed)?;
    }
    let menu = platform.open_window(
        handle(2),
        params(gpui::WindowKind::AnchoredPopup(menu_options)),
    )?;
    let mut nested_options = options(handle(2));
    nested_options.grab = true;
    unsafe {
        SetKeyboardState(&pressed)?;
    }
    let nested = platform.open_window(
        handle(3),
        params(gpui::WindowKind::AnchoredPopup(nested_options)),
    )?;
    let nested_hwnd = nested.get_raw_handle();
    let menu_hwnd = menu.get_raw_handle();
    let close_queued = |hwnd| {
        let mut msg = MSG::default();
        assert!(
            unsafe { PeekMessageW(&mut msg, Some(hwnd), WM_CLOSE, WM_CLOSE, PM_REMOVE) }.as_bool()
        );
        unsafe { DispatchMessageW(&msg) };
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
    };
    unsafe {
        SendMessageW(
            nested_hwnd,
            crate::WM_GPUI_KEYDOWN,
            Some(windows::Win32::Foundation::WPARAM(VK_ESCAPE.0 as usize)),
            None,
        );
    }
    close_queued(nested_hwnd);
    assert!(unsafe { IsWindow(Some(menu_hwnd)) }.as_bool());
    assert!(unsafe { IsWindow(Some(parent.get_raw_handle())) }.as_bool());
    unsafe {
        SendMessageW(menu_hwnd, WM_ACTIVATEAPP, None, None);
    }
    close_queued(menu_hwnd);
    unsafe {
        DestroyWindow(parent.get_raw_handle())?;
    }
    Ok(())
}
