use super::*;
use gpui::popup::{PopupAnchor, PopupConstraintAdjustment as C, PopupGravity, PopupOptions};

pub(super) struct PopupState {
    options: PopupOptions,
    parent: Weak<Mutex<MacWindowState>>,
    observer: id,
    requested_size: NSSize,
}
impl Drop for PopupState {
    fn drop(&mut self) {
        if self.observer.is_null() {
            return;
        }
        unsafe {
            let center: id = msg_send![class!(NSNotificationCenter), defaultCenter];
            let _: () = msg_send![center, removeObserver: self.observer];
            let _: () = msg_send![self.observer, release];
        }
    }
}

pub(super) fn parent(options: &PopupOptions) -> anyhow::Result<Arc<Mutex<MacWindowState>>> {
    unsafe {
        let app = NSApplication::sharedApplication(nil);
        let windows: id = msg_send![app, windows];
        for ix in 0..NSArray::count(windows) {
            let window = NSArray::objectAtIndex(windows, ix);
            if msg_send![window, isKindOfClass: WINDOW_CLASS]
                || msg_send![window, isKindOfClass: PANEL_CLASS]
            {
                let state = get_window_state(&*window);
                let lock = state.lock();
                if lock.handle == options.parent && !lock.closed.load(Ordering::Acquire) {
                    drop(lock);
                    return Ok(state);
                }
            }
        }
    }
    anyhow::bail!("popup parent is not an open macOS window")
}

fn anchor(value: PopupAnchor) -> [f64; 2] {
    use PopupAnchor::*;
    match value {
        Center => [0.5, 0.5],
        Top => [0.5, 0.],
        Bottom => [0.5, 1.],
        Left => [0., 0.5],
        Right => [1., 0.5],
        TopLeft => [0., 0.],
        TopRight => [1., 0.],
        BottomLeft => [0., 1.],
        BottomRight => [1., 1.],
    }
}
fn gravity(value: PopupGravity) -> [f64; 2] {
    use PopupGravity::*;
    match value {
        Center => [0.5, 0.5],
        Top => [0.5, 0.],
        Bottom => [0.5, 1.],
        Left => [0., 0.5],
        Right => [1., 0.5],
        TopLeft => [0., 0.],
        TopRight => [1., 0.],
        BottomLeft => [0., 1.],
        BottomRight => [1., 1.],
    }
}
fn axis(
    start: f64,
    length: f64,
    mut extent: f64,
    a: f64,
    g: f64,
    offset: f64,
    min: f64,
    max: f64,
    flip: bool,
    slide: bool,
    resize: bool,
) -> (f64, f64) {
    let mut position = start + length * a - extent * (1. - g) + offset;
    let fits = |p: f64| p >= min && p + extent <= max;
    if !fits(position) && flip {
        let flipped = start + length * (1. - a) - extent * g + offset;
        if fits(flipped) {
            position = flipped;
        }
    }
    if slide {
        position = position.min(max - extent).max(min);
    }
    if resize {
        let end = (position + extent).min(max);
        position = position.max(min).min(max - 1.);
        extent = (end - position).max(1.);
    }
    (position, extent)
}

pub(super) fn frame(parent: &MacWindowState, options: &PopupOptions, size: NSSize) -> NSRect {
    unsafe {
        let anchor_rect = options.anchor_rect;
        let view = parent.native_view.as_ptr();
        let rect = NSRect::new(
            NSPoint::new(
                anchor_rect.origin.x.to_f64(),
                (parent.content_size().height - anchor_rect.origin.y - anchor_rect.size.height)
                    .to_f64(),
            ),
            NSSize::new(
                anchor_rect.size.width.to_f64(),
                anchor_rect.size.height.to_f64(),
            ),
        );
        let rect: NSRect = msg_send![view, convertRect: rect toView: nil];
        let rect: NSRect = msg_send![parent.native_window, convertRectToScreen: rect];
        let screen: id = msg_send![parent.native_window, screen];
        let screen = if screen.is_null() {
            NSScreen::mainScreen(nil)
        } else {
            screen
        };
        let visible: NSRect = msg_send![screen, visibleFrame];
        let a = anchor(options.anchor);
        let g = gravity(options.gravity);
        let c = options.constraint_adjustment;
        // Solve in a top-down screen coordinate system, then return AppKit coordinates.
        let (x, width) = axis(
            rect.origin.x,
            rect.size.width,
            size.width,
            a[0],
            g[0],
            options.offset.x.to_f64(),
            visible.origin.x,
            visible.origin.x + visible.size.width,
            c.contains(C::FLIP_X),
            c.contains(C::SLIDE_X),
            c.contains(C::RESIZE_X),
        );
        let (y, height) = axis(
            -rect.origin.y - rect.size.height,
            rect.size.height,
            size.height,
            a[1],
            g[1],
            options.offset.y.to_f64(),
            -visible.origin.y - visible.size.height,
            -visible.origin.y,
            c.contains(C::FLIP_Y),
            c.contains(C::SLIDE_Y),
            c.contains(C::RESIZE_Y),
        );
        NSRect::new(NSPoint::new(x, -y - height), NSSize::new(width, height))
    }
}

pub(super) fn attach(
    window: &MacWindow,
    parent: Arc<Mutex<MacWindowState>>,
    options: PopupOptions,
) {
    let native = window.0.lock().native_window;
    let parent_native = parent.lock().native_window;
    let weak = Arc::downgrade(&window.0);
    unsafe {
        let _: () = msg_send![parent_native, addChildWindow: native ordered: NSWindowOrderingMode::NSWindowAbove];
        let block = ConcreteBlock::new(move |_: id| {
            if let Some(state) = weak.upgrade() {
                dismiss(&state);
            }
        })
        .copy();
        let center: id = msg_send![class!(NSNotificationCenter), defaultCenter];
        let observer: id = if options.grab {
            msg_send![center, addObserverForName: ns_string("NSApplicationDidResignActiveNotification") object: nil queue: nil usingBlock: &*block]
        } else {
            nil
        };
        retain_native(observer);
        window.0.lock().popup = Some(PopupState {
            options,
            parent: Arc::downgrade(&parent),
            observer,
            requested_size: NSView::bounds(native.contentView()).size,
        });
    }
}

pub(super) fn reposition(state: &Arc<Mutex<MacWindowState>>) {
    let lock = state.lock();
    let Some(popup) = &lock.popup else {
        return;
    };
    let Some(parent) = popup.parent.upgrade() else {
        return;
    };
    let native = lock.native_window;
    let rect = frame(&parent.lock(), &popup.options, popup.requested_size);
    drop(lock);
    unsafe {
        let _: () = msg_send![native, setFrame: rect display: YES];
    }
}

pub(super) fn resize(state: &Arc<Mutex<MacWindowState>>, size: Size<Pixels>) {
    if let Some(popup) = &mut state.lock().popup {
        popup.requested_size = NSSize::new(size.width.to_f64(), size.height.to_f64());
    }
    reposition(state);
}

pub(super) fn reposition_children(state: &Arc<Mutex<MacWindowState>>) {
    let weak = Arc::downgrade(state);
    state
        .lock()
        .foreground_executor
        .spawn(async move {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let lock = state.lock();
            if lock.closed.load(Ordering::Acquire) {
                return;
            }
            let native = lock.native_window;
            drop(lock);
            unsafe {
                let children: id = msg_send![native, childWindows];
                if children.is_null() {
                    return;
                }
                let children: id = msg_send![children, copy];
                for ix in 0..NSArray::count(children) {
                    let child = NSArray::objectAtIndex(children, ix);
                    if msg_send![child, isKindOfClass: PANEL_CLASS] {
                        reposition(&get_window_state(&*child));
                    }
                }
                release_native(children);
            }
        })
        .detach();
}

pub(super) fn grabs(state: &MacWindowState) -> bool {
    state.popup.as_ref().is_some_and(|p| p.options.grab)
}

pub(super) fn dismiss(state: &Arc<Mutex<MacWindowState>>) {
    let lock = state.lock();
    let weak = Arc::downgrade(state);
    lock.foreground_executor
        .spawn(async move {
            if let Some(state) = weak.upgrade() {
                let lock = state.lock();
                if lock.closed.load(Ordering::Acquire) {
                    return;
                }
                let native = lock.native_window;
                drop(lock);
                unsafe {
                    native.close();
                }
            }
        })
        .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placement_constraints_and_negative_screen_origins() {
        assert_eq!(
            axis(80., 10., 30., 1., 1., 0., 0., 100., true, false, false),
            (50., 30.)
        );
        assert_eq!(
            axis(80., 10., 30., 1., 1., 0., 0., 100., false, true, false),
            (70., 30.)
        );
        assert_eq!(
            axis(80., 10., 30., 1., 1., 0., 0., 100., false, false, true),
            (90., 10.)
        );
        assert_eq!(
            axis(-200., 10., 30., 0., 1., 5., -300., 0., false, false, false),
            (-195., 30.)
        );
        assert_eq!(
            axis(5., 10., 200., 0., 1., 0., 0., 100., false, true, true),
            (0., 100.)
        );
    }
}
