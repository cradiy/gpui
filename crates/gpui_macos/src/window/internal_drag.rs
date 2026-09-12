use super::*;
use gpui::{DragSessionId, InternalDragEvent, Scene};
use std::cell::RefCell;

pub(super) const PASTEBOARD_TYPE: &str = "org.gpui.internal-drag";
const MOVE: NSDragOperation = 16;
thread_local! { static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) }; }

struct Drag {
    id: DragSessionId,
    source: Weak<Mutex<MacWindowState>>,
    item: id,
    session: id,
    image: id,
    logical_size: Size<Pixels>,
    hotspot: Point<Pixels>,
    renderer: crate::metal_renderer::MetalRenderer,
    cancelled: bool,
}
impl Drop for Drag {
    fn drop(&mut self) {
        unsafe {
            release_native(self.item);
            release_native(self.session);
            release_native(self.image);
        }
    }
}

pub(super) fn register(decl: &mut ClassDecl) {
    unsafe {
        decl.add_protocol(Protocol::get("NSDraggingSource").unwrap());
        decl.add_method(
            sel!(draggingSession:sourceOperationMaskForDraggingContext:),
            operations as extern "C" fn(&Object, Sel, id, NSInteger) -> NSDragOperation,
        );
        decl.add_method(
            sel!(draggingSession:endedAtPoint:operation:),
            ended as extern "C" fn(&Object, Sel, id, NSPoint, NSDragOperation),
        );
        decl.add_method(
            sel!(ignoreModifierKeysForDraggingSession:),
            ignore_modifiers as extern "C" fn(&Object, Sel, id) -> BOOL,
        );
    }
}

fn send(state: &Arc<Mutex<MacWindowState>>, event: InternalDragEvent) -> gpui::DispatchEventResult {
    let callback = state.lock().event_callback.take();
    if let Some(mut callback) = callback {
        let result = callback(PlatformInput::InternalDrag(event));
        state.lock().event_callback = Some(callback);
        result
    } else {
        Default::default()
    }
}

pub(super) fn create(
    state: &Arc<Mutex<MacWindowState>>,
    session_id: DragSessionId,
    logical_size: Size<Pixels>,
    scale: f32,
    hotspot: Point<Pixels>,
    scene: &Scene,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        DRAG.with(|slot| slot.borrow().is_none()),
        "a macOS drag is already active"
    );
    let renderer = state.lock().renderer.new_auxiliary();
    DRAG.with(|slot| {
        *slot.borrow_mut() = Some(Drag {
            id: session_id,
            source: Arc::downgrade(state),
            item: nil,
            session: nil,
            image: nil,
            logical_size,
            hotspot,
            renderer,
            cancelled: false,
        });
    });
    if let Err(error) = update(session_id, logical_size, scale, hotspot, scene) {
        destroy(session_id);
        return Err(error);
    }
    Ok(())
}

pub(super) fn update(
    session_id: DragSessionId,
    logical_size: Size<Pixels>,
    scale: f32,
    hotspot: Point<Pixels>,
    scene: &Scene,
) -> anyhow::Result<()> {
    DRAG.with(|slot| {
        let mut slot = slot.borrow_mut();
        let drag = slot.as_mut().filter(|drag| drag.id == session_id).ok_or_else(|| anyhow::anyhow!("drag icon does not exist"))?;
        anyhow::ensure!(scale.is_finite() && scale > 0., "invalid drag icon scale");
        let image = drag.renderer.render_scene_to_image(scene, logical_size.map(|p| gpui::DevicePixels((p.as_f32() * scale).ceil() as i32)))?;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png)?;
        let bytes = bytes.into_inner();
        unsafe {
            let data: id = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr() length: bytes.len()];
            let image: id = msg_send![class!(NSImage), alloc];
            let image: id = msg_send![image, initWithData: data];
            anyhow::ensure!(!image.is_null(), "failed to create drag image");
            let _: () = msg_send![image, setSize: NSSize::new(logical_size.width.to_f64(), logical_size.height.to_f64())];
            release_native(drag.image);
            drag.image = image;
            if !drag.item.is_null() {
                let frame: NSRect = msg_send![drag.item, draggingFrame];
                let rect = updated_image_frame(frame, drag.hotspot, logical_size, hotspot);
                let _: () = msg_send![drag.item, setDraggingFrame: rect contents: image];
            }
            drag.logical_size = logical_size;
            drag.hotspot = hotspot;
        }
        Ok(())
    })
}

fn updated_image_frame(
    frame: NSRect,
    old_hotspot: Point<Pixels>,
    size: Size<Pixels>,
    hotspot: Point<Pixels>,
) -> NSRect {
    NSRect::new(
        NSPoint::new(
            frame.origin.x + old_hotspot.x.to_f64() - hotspot.x.to_f64(),
            frame.origin.y + frame.size.height - old_hotspot.y.to_f64() - size.height.to_f64()
                + hotspot.y.to_f64(),
        ),
        NSSize::new(size.width.to_f64(), size.height.to_f64()),
    )
}

pub(super) fn start(
    state: &Arc<Mutex<MacWindowState>>,
    session_id: DragSessionId,
    has_icon: bool,
) -> anyhow::Result<()> {
    let (view, event) = {
        let lock = state.lock();
        (lock.native_view.as_ptr(), lock.drag_event)
    };
    anyhow::ensure!(
        !event.is_null(),
        "native drag requires an active mouse event"
    );
    let event_type: NSUInteger = unsafe { msg_send![event, type] };
    anyhow::ensure!(
        event_type == 1 || event_type == 6,
        "native drag requires a left mouse press or drag event"
    );
    if !has_icon {
        create(
            state,
            session_id,
            size(px(1.), px(1.)),
            1.,
            point(px(0.), px(0.)),
            &Scene::default(),
        )?;
    }
    let item = DRAG.with(|slot| -> anyhow::Result<id> {
        let mut slot = slot.borrow_mut();
        let drag = slot.as_mut().filter(|d| d.id == session_id).ok_or_else(|| anyhow::anyhow!("drag icon does not exist"))?;
        anyhow::ensure!(drag.session.is_null(), "drag session already started");
        unsafe {
            let writer: id = msg_send![class!(NSPasteboardItem), new];
            let _: BOOL = msg_send![writer, setString: ns_string(&session_id.as_u64().to_string()) forType: ns_string(PASTEBOARD_TYPE)];
            let item: id = msg_send![class!(NSDraggingItem), alloc];
            let item: id = msg_send![item, initWithPasteboardWriter: writer];
            let _: () = msg_send![writer, release];
            let location: NSPoint = msg_send![event, locationInWindow];
            let location: NSPoint = msg_send![view, convertPoint: location fromView: nil];
            let rect = NSRect::new(NSPoint::new(location.x - drag.hotspot.x.to_f64(), location.y - drag.logical_size.height.to_f64() + drag.hotspot.y.to_f64()), NSSize::new(drag.logical_size.width.to_f64(), drag.logical_size.height.to_f64()));
            let _: () = msg_send![item, setDraggingFrame: rect contents: drag.image];
            drag.item = item;
            Ok(item)
        }
    })?;
    // AppKit can synchronously call NSDraggingSource while starting the session.
    // Never hold the process-local registry borrow or window mutex across this call.
    unsafe {
        let items = NSArray::arrayWithObject(nil, item);
        let session: id =
            msg_send![view, beginDraggingSessionWithItems: items event: event source: view];
        if session.is_null() {
            destroy(session_id);
            anyhow::bail!("AppKit refused the drag session");
        }
        let _: () = msg_send![session, setAnimatesToStartingPositionsOnCancelOrFail: NO];
        DRAG.with(|slot| {
            if let Some(drag) = slot.borrow_mut().as_mut().filter(|d| d.id == session_id) {
                let _: () = msg_send![session, retain];
                drag.session = session;
            }
        });
    }
    state.lock().synthetic_drag_counter += 1;
    Ok(())
}

pub(super) fn destroy(session_id: DragSessionId) {
    DRAG.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|d| d.id == session_id && d.session.is_null())
        {
            slot.take();
        }
    });
}

pub(super) fn cancel(session_id: DragSessionId) {
    let source = DRAG.with(|slot| {
        let mut slot = slot.borrow_mut();
        let drag = slot
            .as_mut()
            .filter(|d| d.id == session_id && !d.cancelled)?;
        drag.cancelled = true;
        if drag.session.is_null() {
            return None;
        }
        drag.source.upgrade()
    });
    if let Some(source) = source {
        // AppKit's public drag loop handles Escape, including image and cursor cleanup.
        let lock = source.lock();
        let closed = lock.closed.clone();
        let window = lock.native_window;
        lock.foreground_executor.spawn(async move {
            if !DRAG.with(|slot| slot.borrow().as_ref().is_some_and(|d| d.id == session_id && d.cancelled && !d.session.is_null())) { return; }
            if_window_not_closed(closed, || unsafe {
            let app = NSApplication::sharedApplication(nil);
            let number: NSInteger = msg_send![window, windowNumber];
            let event: id = msg_send![class!(NSEvent), keyEventWithType: 10u64 location: NSPoint::new(0.,0.) modifierFlags: 0u64 timestamp: 0f64 windowNumber: number context: nil characters: ns_string("\u{1b}") charactersIgnoringModifiers: ns_string("\u{1b}") isARepeat: NO keyCode: 53u16];
            let _: () = msg_send![app, postEvent: event atStart: YES];
        }); }).detach();
    } else {
        destroy(session_id);
    }
}

pub(super) fn session(info: id) -> Option<DragSessionId> {
    let source: id = unsafe { msg_send![info, draggingSource] };
    DRAG.with(|slot| {
        let slot = slot.borrow();
        let drag = slot.as_ref()?;
        let state = drag.source.upgrade()?;
        // The pasteboard is untrusted. Only this process's active source view can
        // identify a GPUI session; external strings never become session IDs.
        (state.lock().native_view.as_ptr() == source).then_some(drag.id)
    })
}

pub(super) fn target(
    state: &Arc<Mutex<MacWindowState>>,
    event: InternalDragEvent,
) -> NSDragOperation {
    if DRAG.with(|slot| slot.borrow().as_ref().is_none_or(|d| d.cancelled)) {
        return NSDragOperationNone;
    }
    send(state, event);
    MOVE
}

pub(super) fn dropped(
    state: &Arc<Mutex<MacWindowState>>,
    session_id: DragSessionId,
    position: Point<Pixels>,
) -> BOOL {
    if DRAG.with(|slot| slot.borrow().as_ref().is_none_or(|d| d.cancelled)) {
        return NO;
    }
    let accepted = send(
        state,
        InternalDragEvent::Dropped {
            session_id,
            position,
        },
    )
    .drag_drop_accepted;
    let source = DRAG.with(|slot| slot.borrow().as_ref().and_then(|d| d.source.upgrade()));
    if let Some(source) = source {
        send(
            &source,
            InternalDragEvent::SourceDropPerformed { session_id },
        );
    }
    accepted.to_objc()
}

extern "C" fn operations(_: &Object, _: Sel, _: id, context: NSInteger) -> NSDragOperation {
    if context == 1 && DRAG.with(|slot| slot.borrow().as_ref().is_some_and(|d| !d.cancelled)) {
        MOVE
    } else {
        NSDragOperationNone
    }
}
extern "C" fn ignore_modifiers(_: &Object, _: Sel, _: id) -> BOOL {
    YES
}
extern "C" fn ended(_: &Object, _: Sel, _: id, _: NSPoint, operation: NSDragOperation) {
    let drag = DRAG.with(|slot| slot.borrow_mut().take());
    if let Some(drag) = drag {
        if let Some(source) = drag.source.upgrade() {
            let event = if operation == MOVE && !drag.cancelled {
                InternalDragEvent::SourceFinished {
                    session_id: drag.id,
                }
            } else {
                InternalDragEvent::SourceCancelled {
                    session_id: drag.id,
                }
            };
            send(&source, event);
        }
    }
}

pub(super) fn source_closed(state: &Arc<Mutex<MacWindowState>>) {
    let session = DRAG.with(|slot| {
        slot.borrow()
            .as_ref()
            .filter(|d| d.source.ptr_eq(&Arc::downgrade(state)))
            .map(|d| d.id)
    });
    if let Some(session) = session {
        cancel(session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_icon_size_preserves_pointer_anchor() {
        let frame = NSRect::new(NSPoint::new(100., 200.), NSSize::new(80., 40.));
        let resized = updated_image_frame(
            frame,
            point(px(10.), px(12.)),
            size(px(120.), px(60.)),
            point(px(20.), px(15.)),
        );
        assert_eq!((resized.origin.x, resized.origin.y), (90., 183.));
        let restored = updated_image_frame(
            resized,
            point(px(20.), px(15.)),
            size(px(80.), px(40.)),
            point(px(10.), px(12.)),
        );
        assert_eq!(
            (
                restored.origin.x,
                restored.origin.y,
                restored.size.width,
                restored.size.height
            ),
            (100., 200., 80., 40.)
        );
    }

    #[test]
    fn empty_native_objects_can_be_released() {
        unsafe {
            retain_native(nil);
            release_native(nil);
        }
    }
}
