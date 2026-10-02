use super::super::window::drag_icon::X11DragIcon;
use super::*;
use calloop::timer::{TimeoutAction, Timer};
use gpui::{
    DragAction, DragFailure, DragSessionId, InternalDragEvent as DragEvent, SystemFileDrag,
};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    window: u32,
    proxy: u32,
    version: u32,
    internal: bool,
}

struct Transfer {
    requestor: u32,
    property: u32,
    offset: usize,
    deadline: Instant,
}

pub(super) struct NativeDrag {
    session: DragSessionId,
    pub(super) source: X11WindowStatePtr,
    connection: Rc<XCBConnection>,
    atoms: XcbAtoms,
    files: Option<SystemFileDrag>,
    icon: Option<X11DragIcon>,
    target: Option<Target>,
    action: Option<DragAction>,
    position: Option<(i16, i16, DragAction)>,
    pending_position: Option<(i16, i16, DragAction)>,
    waiting_status: bool,
    released: bool,
    dropped: bool,
    deadline: Option<Instant>,
    transfers: Vec<Transfer>,
    watches: HashMap<u32, EventMask>,
    handle: LoopHandle<'static, X11Client>,
    timer: Option<RegistrationToken>,
    grabbed: bool,
}

impl Drop for NativeDrag {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            self.handle.remove(timer);
        }
        if self.grabbed {
            let _ = self.connection.ungrab_pointer(x11rb::CURRENT_TIME);
            let _ = self.connection.ungrab_keyboard(x11rb::CURRENT_TIME);
        }
        for (window, mask) in &self.watches {
            let _ = self.connection.change_window_attributes(
                *window,
                &ChangeWindowAttributesAux::new().event_mask(*mask),
            );
        }
        let _ = self
            .connection
            .delete_property(self.source.x_window, self.atoms.XdndActionList);
        // A cleared selection also terminates any outstanding INCR transfers.
        if self
            .connection
            .get_selection_owner(self.atoms.XdndSelection)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|r| r.owner == self.source.x_window)
        {
            let _ = self.connection.set_selection_owner(
                x11rb::NONE,
                self.atoms.XdndSelection,
                x11rb::CURRENT_TIME,
            );
        }
        let _ = self.connection.flush();
    }
}

fn atom_for(atoms: &XcbAtoms, action: DragAction) -> u32 {
    match action {
        DragAction::Copy => atoms.XdndActionCopy,
        DragAction::Move => atoms.XdndActionMove,
        DragAction::Link => atoms.XdndActionLink,
    }
}
fn action_for(atoms: &XcbAtoms, atom: u32) -> Option<DragAction> {
    [DragAction::Copy, DragAction::Move, DragAction::Link]
        .into_iter()
        .find(|action| atom_for(atoms, *action) == atom)
}
fn message(
    connection: &XCBConnection,
    target: Target,
    kind: u32,
    data: [u32; 5],
) -> anyhow::Result<()> {
    connection
        .send_event(
            false,
            target.proxy,
            EventMask::NO_EVENT,
            ClientMessageEvent::new(32, target.window, kind, data),
        )?
        .check()?;
    Ok(())
}

impl X11ClientStatePtr {
    pub(crate) fn create_drag_icon(
        &self,
        session: DragSessionId,
        size: Size<Pixels>,
        scale: f32,
        hotspot: Point<Pixels>,
        scene: &gpui::Scene,
    ) -> anyhow::Result<()> {
        let client = self
            .get_client()
            .ok_or_else(|| anyhow!("X11 client closed"))?;
        let mut state = client.0.borrow_mut();
        let icon = X11DragIcon::new(
            state.xcb_connection.clone(),
            state.x_root_index,
            state.gpu_context.clone(),
            state.compositor_gpu,
            size,
            scale,
            hotspot,
            scene,
        )?;
        state.pending_drag_icons.insert(session, icon);
        Ok(())
    }
    pub(crate) fn update_drag_icon(
        &self,
        session: DragSessionId,
        size: Size<Pixels>,
        scale: f32,
        hotspot: Point<Pixels>,
        scene: &gpui::Scene,
    ) -> anyhow::Result<()> {
        let client = self
            .get_client()
            .ok_or_else(|| anyhow!("X11 client closed"))?;
        let mut state = client.0.borrow_mut();
        if let Some(icon) = state
            .native_drag
            .as_mut()
            .filter(|drag| drag.session == session)
            .and_then(|drag| drag.icon.as_mut())
        {
            icon.draw(size, scale, hotspot, scene)
        } else if let Some(icon) = state.pending_drag_icons.get_mut(&session) {
            icon.draw(size, scale, hotspot, scene)
        } else {
            Ok(())
        }
    }
    pub(crate) fn destroy_drag_icon(&self, session: DragSessionId) {
        if let Some(client) = self.get_client() {
            let mut state = client.0.borrow_mut();
            state.pending_drag_icons.remove(&session);
            if let Some(drag) = state
                .native_drag
                .as_mut()
                .filter(|drag| drag.session == session)
            {
                drag.icon = None;
            }
        }
    }
    pub(crate) fn cancel_drag(&self, session: DragSessionId) {
        if let Some(client) = self.get_client() {
            let drag = {
                let mut state = client.0.borrow_mut();
                if state
                    .native_drag
                    .as_ref()
                    .is_some_and(|drag| drag.session == session)
                {
                    state.native_drag.take()
                } else {
                    None
                }
            };
            if let Some(drag) = drag {
                // Cancellation is called from inside GPUI's update. Do not reenter
                // an application callback while its App is already borrowed.
                if let Some(target) = drag
                    .target
                    .filter(|target| !target.internal && !drag.dropped)
                {
                    message(
                        &drag.connection,
                        target,
                        drag.atoms.XdndLeave,
                        [drag.source.x_window, 0, 0, 0, 0],
                    )
                    .log_err();
                }
            }
        }
    }
    pub(crate) fn start_drag(
        &self,
        source: X11WindowStatePtr,
        session: DragSessionId,
        has_icon: bool,
        files: Option<SystemFileDrag>,
    ) -> anyhow::Result<()> {
        let client = self
            .get_client()
            .ok_or_else(|| anyhow!("X11 client closed"))?;
        let mut state = client.0.borrow_mut();
        anyhow::ensure!(state.native_drag.is_none(), "another X11 drag is active");
        let connection = state.xcb_connection.clone();
        let root = connection.setup().roots[state.x_root_index].root;
        let pointer = connection.query_pointer(root)?.reply()?;
        anyhow::ensure!(
            pointer.mask.contains(xproto::KeyButMask::BUTTON1),
            "file drag requires an initiating left button press"
        );
        let mut drag = NativeDrag {
            session,
            source,
            connection: connection.clone(),
            atoms: state.atoms,
            files,
            icon: if has_icon {
                Some(
                    state
                        .pending_drag_icons
                        .remove(&session)
                        .ok_or_else(|| anyhow!("drag icon not created"))?,
                )
            } else {
                None
            },
            target: None,
            action: None,
            position: None,
            pending_position: None,
            waiting_status: false,
            released: false,
            dropped: false,
            deadline: None,
            transfers: Vec::new(),
            watches: HashMap::default(),
            handle: state.loop_handle.clone(),
            timer: None,
            grabbed: false,
        };
        // Grabs use the root so hiding the source window does not release them.
        let status = connection
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
                x11rb::NONE,
                x11rb::NONE,
                x11rb::CURRENT_TIME,
            )?
            .reply()?
            .status;
        anyhow::ensure!(
            status == xproto::GrabStatus::SUCCESS,
            "X11 pointer grab failed: {status:?}"
        );
        drag.grabbed = true;
        let status = connection
            .grab_keyboard(
                false,
                root,
                x11rb::CURRENT_TIME,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
            )?
            .reply()?
            .status;
        anyhow::ensure!(
            status == xproto::GrabStatus::SUCCESS,
            "X11 keyboard grab failed: {status:?}"
        );
        connection
            .set_selection_owner(
                drag.source.x_window,
                state.atoms.XdndSelection,
                x11rb::CURRENT_TIME,
            )?
            .check()?;
        anyhow::ensure!(
            connection
                .get_selection_owner(state.atoms.XdndSelection)?
                .reply()?
                .owner
                == drag.source.x_window,
            "cannot own XdndSelection"
        );
        if let Some(files) = &drag.files {
            let actions: Vec<_> = [DragAction::Copy, DragAction::Move, DragAction::Link]
                .into_iter()
                .filter(|a| files.options().allowed_actions.allows(*a))
                .map(|a| atom_for(&state.atoms, a))
                .collect();
            connection
                .change_property32(
                    xproto::PropMode::REPLACE,
                    drag.source.x_window,
                    state.atoms.XdndActionList,
                    AtomEnum::ATOM,
                    &actions,
                )?
                .check()?;
        }
        drag.timer = Some(
            state
                .loop_handle
                .insert_source(
                    Timer::from_duration(Duration::from_millis(1)),
                    move |_, _, client| {
                        if let Err(error) = client.poll_native_drag(session) {
                            log::warn!("X11 file drag failed: {error:#}");
                            client.end_native_drag(
                                session,
                                DragEvent::SourceFailed {
                                    session_id: session,
                                    failure: DragFailure::Protocol,
                                },
                            );
                        }
                        if client
                            .0
                            .borrow()
                            .native_drag
                            .as_ref()
                            .is_some_and(|drag| drag.session == session)
                        {
                            TimeoutAction::ToDuration(Duration::from_millis(16))
                        } else {
                            TimeoutAction::Drop
                        }
                    },
                )
                .map_err(|error| anyhow!("cannot register XDND timer: {error}"))?,
        );
        state.native_drag = Some(drag);
        connection.flush()?;
        Ok(())
    }
}

impl X11Client {
    fn leave_drag_target(&self, drag: &NativeDrag) {
        if let Some(target) = drag.target {
            if target.internal {
                if let Some(window) = self.get_window(target.window) {
                    window.handle_drag(DragEvent::Left {
                        session_id: drag.session,
                    });
                }
            } else if !drag.dropped {
                message(
                    &drag.connection,
                    target,
                    drag.atoms.XdndLeave,
                    [drag.source.x_window, 0, 0, 0, 0],
                )
                .log_err();
            }
        }
    }
    fn end_native_drag(&self, session: DragSessionId, event: DragEvent) {
        let drag = {
            let mut state = self.0.borrow_mut();
            if state
                .native_drag
                .as_ref()
                .is_some_and(|drag| drag.session == session)
            {
                state.native_drag.take()
            } else {
                None
            }
        };
        if let Some(drag) = drag {
            self.leave_drag_target(&drag);
            let source = drag.source.clone();
            drop(drag);
            source.handle_drag(event);
        }
    }

    fn target_at_pointer(
        &self,
        connection: &XCBConnection,
        root: u32,
        atoms: &XcbAtoms,
    ) -> anyhow::Result<Option<Target>> {
        let mut current = root;
        let mut result = None;
        for _ in 0..32 {
            let child = connection.query_pointer(current)?.reply()?.child;
            if child == 0 || child == current {
                break;
            }
            current = child;
            if self.get_window(current).is_some() {
                result = Some(Target {
                    window: current,
                    proxy: current,
                    version: 5,
                    internal: true,
                });
            } else {
                let property = |window, name| -> anyhow::Result<Option<u32>> {
                    Ok(connection
                        .get_property(false, window, name, AtomEnum::ANY, 0, 1)?
                        .reply()?
                        .value32()
                        .and_then(|mut v| v.next()))
                };
                let proxy = property(current, atoms.XdndProxy)?
                    .filter(|proxy| {
                        property(*proxy, atoms.XdndProxy).ok().flatten() == Some(*proxy)
                    })
                    .unwrap_or(current);
                if let Some(version) = property(proxy, atoms.XdndAware)? {
                    if version >= 3 {
                        result = Some(Target {
                            window: current,
                            proxy,
                            version: version.min(5),
                            internal: false,
                        });
                    }
                }
            }
        }
        Ok(result)
    }

    fn poll_native_drag(&self, session: DragSessionId) -> anyhow::Result<()> {
        let (connection, root, atoms, source, expired, released) = {
            let state = self.0.borrow();
            let Some(drag) = state
                .native_drag
                .as_ref()
                .filter(|drag| drag.session == session)
            else {
                return Ok(());
            };
            (
                state.xcb_connection.clone(),
                state.xcb_connection.setup().roots[state.x_root_index].root,
                state.atoms,
                drag.source.clone(),
                drag.deadline
                    .is_some_and(|deadline| Instant::now() >= deadline)
                    || drag.transfers.iter().any(|t| Instant::now() >= t.deadline),
                drag.released,
            )
        };
        if expired {
            self.end_native_drag(
                session,
                DragEvent::SourceFailed {
                    session_id: session,
                    failure: DragFailure::TimedOut,
                },
            );
            return Ok(());
        }
        if released {
            return Ok(());
        }
        let pointer = connection.query_pointer(root)?.reply()?;
        let target = self.target_at_pointer(&connection, root, &atoms)?;
        let (old_target, changed, position, request, files) = {
            let mut state = self.0.borrow_mut();
            let drag = state.native_drag.as_mut().unwrap();
            if let Some(icon) = &drag.icon {
                icon.move_to(pointer.root_x, pointer.root_y)?;
            }
            let request = drag.files.as_ref().map_or(DragAction::Move, |files| {
                files
                    .options()
                    .action_for_modifiers(modifiers_from_state(pointer.mask))
            });
            let position = (pointer.root_x, pointer.root_y, request);
            let changed = drag.position != Some(position);
            let old = drag.target;
            if old != target {
                drag.action = None;
                drag.waiting_status = false;
                drag.position = None;
                drag.pending_position = None;
                drag.deadline = None;
            }
            drag.target = target;
            if target.is_some_and(|target| target.internal) {
                drag.position = Some(position);
            }
            (old, changed, position, request, drag.files.is_some())
        };
        if old_target != target {
            if let Some(old) = old_target {
                if old.internal {
                    if let Some(window) = self.get_window(old.window) {
                        window.handle_drag(DragEvent::Left {
                            session_id: session,
                        });
                    }
                } else {
                    message(
                        &connection,
                        old,
                        atoms.XdndLeave,
                        [source.x_window, 0, 0, 0, 0],
                    )?;
                }
            }
            if !self
                .0
                .borrow()
                .native_drag
                .as_ref()
                .is_some_and(|drag| drag.session == session)
            {
                return Ok(());
            }
            if let Some(target) = target.filter(|t| !t.internal && files) {
                message(
                    &connection,
                    target,
                    atoms.XdndEnter,
                    [
                        source.x_window,
                        target.version << 24,
                        atoms.TextUriList,
                        0,
                        0,
                    ],
                )?;
            }
        }
        if let Some(target) = target {
            if target.internal {
                if let Some(window) = self.get_window(target.window) {
                    let translated = connection
                        .translate_coordinates(root, target.window, pointer.root_x, pointer.root_y)?
                        .reply()?;
                    let scale = self.0.borrow().scale_factor;
                    let local = point(
                        px(translated.dst_x as f32 / scale),
                        px(translated.dst_y as f32 / scale),
                    );
                    if old_target != Some(target) {
                        window.handle_drag(DragEvent::Entered {
                            session_id: session,
                            position: local,
                        });
                    }
                    if changed {
                        window.handle_drag(DragEvent::Moved {
                            session_id: session,
                            position: local,
                        });
                    }
                    if !pointer.mask.contains(xproto::KeyButMask::BUTTON1) {
                        source.handle_drag(DragEvent::SourceDropPerformed {
                            session_id: session,
                        });
                        window.handle_drag(DragEvent::Dropped {
                            session_id: session,
                            position: local,
                        });
                        self.end_native_drag(
                            session,
                            DragEvent::SourceFinished {
                                session_id: session,
                                action: None,
                            },
                        );
                        return Ok(());
                    }
                }
            } else if files {
                let mut state = self.0.borrow_mut();
                let Some(drag) = state
                    .native_drag
                    .as_mut()
                    .filter(|drag| drag.session == session)
                else {
                    return Ok(());
                };
                if drag.waiting_status && changed {
                    drag.pending_position = Some(position);
                }
                if !drag.waiting_status && (changed || old_target != Some(target)) {
                    message(
                        &connection,
                        target,
                        atoms.XdndPosition,
                        [
                            source.x_window,
                            0,
                            ((pointer.root_x as u16 as u32) << 16) | pointer.root_y as u16 as u32,
                            x11rb::CURRENT_TIME,
                            atom_for(&atoms, request),
                        ],
                    )?;
                    drag.waiting_status = true;
                    drag.position = Some(position);
                    drag.deadline = Some(Instant::now() + Duration::from_secs(5));
                }
            }
        }
        if !pointer.mask.contains(xproto::KeyButMask::BUTTON1) {
            let mut state = self.0.borrow_mut();
            let Some(drag) = state
                .native_drag
                .as_mut()
                .filter(|drag| drag.session == session)
            else {
                return Ok(());
            };
            drag.released = true;
            drag.icon = None;
            connection.ungrab_pointer(x11rb::CURRENT_TIME)?;
            connection.ungrab_keyboard(x11rb::CURRENT_TIME)?;
            drag.grabbed = false;
            let waiting = drag.waiting_status;
            drop(state);
            source.handle_drag(DragEvent::SourceDropPerformed {
                session_id: session,
            });
            if !waiting {
                self.drop_native_drag(session)?;
            }
        }
        connection.flush()?;
        Ok(())
    }

    fn drop_native_drag(&self, session: DragSessionId) -> anyhow::Result<()> {
        let mut state = self.0.borrow_mut();
        let Some(drag) = state
            .native_drag
            .as_mut()
            .filter(|drag| drag.session == session)
        else {
            return Ok(());
        };
        if let Some(target) = drag.target.filter(|_| drag.action.is_some()) {
            message(
                &drag.connection,
                target,
                drag.atoms.XdndDrop,
                [drag.source.x_window, 0, x11rb::CURRENT_TIME, 0, 0],
            )?;
            drag.dropped = true;
            drag.deadline = Some(Instant::now() + Duration::from_secs(60));
            drag.connection.flush()?;
        } else {
            drop(state);
            self.end_native_drag(
                session,
                DragEvent::SourceFinished {
                    session_id: session,
                    action: None,
                },
            );
        }
        Ok(())
    }

    pub(super) fn handle_native_drag_event(&self, event: &Event) -> bool {
        let session = self
            .0
            .borrow()
            .native_drag
            .as_ref()
            .map(|drag| drag.session);
        let Some(session) = session else {
            // A selection request may already be queued when the drag is cancelled.
            if let Event::SelectionRequest(request) = event {
                let state = self.0.borrow();
                if request.selection == state.atoms.XdndSelection {
                    let reply = xproto::SelectionNotifyEvent {
                        response_type: xproto::SELECTION_NOTIFY_EVENT,
                        sequence: 0,
                        time: request.time,
                        requestor: request.requestor,
                        selection: request.selection,
                        target: request.target,
                        property: x11rb::NONE,
                    };
                    if let Ok(cookie) = state.xcb_connection.send_event(
                        false,
                        request.requestor,
                        EventMask::NO_EVENT,
                        reply,
                    ) {
                        let _ = cookie.check();
                    }
                    let _ = state.xcb_connection.flush();
                    return true;
                }
            }
            return false;
        };
        match self.native_drag_event(event, session) {
            Ok(handled) => handled,
            Err(error) => {
                log::warn!("XDND operation failed: {error:#}");
                self.end_native_drag(
                    session,
                    DragEvent::SourceFailed {
                        session_id: session,
                        failure: DragFailure::Protocol,
                    },
                );
                true
            }
        }
    }

    fn native_drag_event(&self, event: &Event, session: DragSessionId) -> anyhow::Result<bool> {
        let mut state = self.0.borrow_mut();
        let atoms = state.atoms;
        let connection = state.xcb_connection.clone();
        if let Event::KeyPress(key) = event {
            let escape = state.xkb.key_get_one_sym(key.detail.into()) == xkbc::Keysym::Escape;
            drop(state);
            if escape {
                self.end_native_drag(
                    session,
                    DragEvent::SourceCancelled {
                        session_id: session,
                    },
                );
            }
            return Ok(true);
        }
        let drag = state.native_drag.as_mut().unwrap();
        match event {
            Event::ClientMessage(event)
                if event.window == drag.source.x_window
                    && (event.type_ == atoms.XdndStatus || event.type_ == atoms.XdndFinished) =>
            {
                let data = event.data.as_data32();
                let Some(target) = drag
                    .target
                    .filter(|target| target.window == data[0] || target.proxy == data[0])
                else {
                    return Ok(true);
                };
                let action = action_for(
                    &atoms,
                    if event.type_ == atoms.XdndStatus {
                        data[4]
                    } else {
                        data[2]
                    },
                )
                .filter(|action| {
                    drag.files
                        .as_ref()
                        .is_some_and(|files| files.options().allowed_actions.allows(*action))
                });
                if event.type_ == atoms.XdndStatus && drag.waiting_status && !drag.dropped {
                    drag.waiting_status = false;
                    drag.deadline = None;
                    drag.action = if data[1] & 1 != 0 { action } else { None };
                    if let Some((x, y, action)) = drag.pending_position.take() {
                        message(
                            &connection,
                            target,
                            atoms.XdndPosition,
                            [
                                drag.source.x_window,
                                0,
                                ((x as u16 as u32) << 16) | y as u16 as u32,
                                x11rb::CURRENT_TIME,
                                atom_for(&atoms, action),
                            ],
                        )?;
                        drag.position = Some((x, y, action));
                        drag.waiting_status = true;
                        drag.deadline = Some(Instant::now() + Duration::from_secs(5));
                        connection.flush()?;
                        return Ok(true);
                    }
                    let released = drag.released;
                    drop(state);
                    if released {
                        self.drop_native_drag(session)?;
                    }
                } else if event.type_ == atoms.XdndFinished && drag.dropped {
                    anyhow::ensure!(
                        target.version < 5 || data[1] & 1 == 0 || action.is_some(),
                        "XDND target confirmed an invalid action"
                    );
                    let action = if target.version < 5 {
                        drag.action
                    } else if data[1] & 1 != 0 {
                        action
                    } else {
                        None
                    };
                    drop(state);
                    self.end_native_drag(
                        session,
                        DragEvent::SourceFinished {
                            session_id: session,
                            action,
                        },
                    );
                }
                Ok(true)
            }
            Event::SelectionClear(event) if event.selection == atoms.XdndSelection => {
                drop(state);
                self.end_native_drag(
                    session,
                    DragEvent::SourceCancelled {
                        session_id: session,
                    },
                );
                Ok(true)
            }
            Event::SelectionRequest(event) if event.selection == atoms.XdndSelection => {
                let property = if event.property == 0 {
                    event.target
                } else {
                    event.property
                };
                let bytes = drag.files.as_ref().map(|files| files.uri_list().clone());
                let mut accepted = false;
                if event.owner == drag.source.x_window {
                    if event.target == atoms.TARGETS {
                        connection
                            .change_property32(
                                xproto::PropMode::REPLACE,
                                event.requestor,
                                property,
                                AtomEnum::ATOM,
                                &[atoms.TARGETS, atoms.TextUriList],
                            )?
                            .check()?;
                        accepted = bytes.is_some();
                    } else if event.target == atoms.TextUriList {
                        if let Some(bytes) = bytes {
                            if bytes.len() <= 65536 {
                                connection
                                    .change_property8(
                                        xproto::PropMode::REPLACE,
                                        event.requestor,
                                        property,
                                        atoms.TextUriList,
                                        &bytes,
                                    )?
                                    .check()?;
                            } else {
                                if let std::collections::hash_map::Entry::Vacant(entry) =
                                    drag.watches.entry(event.requestor)
                                {
                                    let mask = connection
                                        .get_window_attributes(event.requestor)?
                                        .reply()?
                                        .your_event_mask;
                                    connection
                                        .change_window_attributes(
                                            event.requestor,
                                            &ChangeWindowAttributesAux::new()
                                                .event_mask(mask | EventMask::PROPERTY_CHANGE),
                                        )?
                                        .check()?;
                                    entry.insert(mask);
                                }
                                anyhow::ensure!(
                                    !drag.transfers.iter().any(|t| t.requestor == event.requestor
                                        && t.property == property),
                                    "duplicate XDND INCR transfer"
                                );
                                connection
                                    .change_property32(
                                        xproto::PropMode::REPLACE,
                                        event.requestor,
                                        property,
                                        atoms.INCR,
                                        &[bytes.len().try_into()?],
                                    )?
                                    .check()?;
                                drag.transfers.push(Transfer {
                                    requestor: event.requestor,
                                    property,
                                    offset: 0,
                                    deadline: Instant::now() + Duration::from_secs(10),
                                });
                            }
                            accepted = true;
                        }
                    }
                }
                let notify = xproto::SelectionNotifyEvent {
                    response_type: xproto::SELECTION_NOTIFY_EVENT,
                    sequence: 0,
                    time: event.time,
                    requestor: event.requestor,
                    selection: event.selection,
                    target: event.target,
                    property: if accepted { property } else { 0 },
                };
                connection
                    .send_event(false, event.requestor, EventMask::NO_EVENT, notify)?
                    .check()?;
                connection.flush()?;
                Ok(true)
            }
            Event::PropertyNotify(event) if event.state == xproto::Property::DELETE => {
                let Some(index) = drag
                    .transfers
                    .iter()
                    .position(|t| t.requestor == event.window && t.property == event.atom)
                else {
                    return Ok(false);
                };
                let bytes: Arc<[u8]> = drag.files.as_ref().unwrap().uri_list().clone();
                let transfer = &mut drag.transfers[index];
                let end = (transfer.offset + 65536).min(bytes.len());
                connection
                    .change_property8(
                        xproto::PropMode::REPLACE,
                        event.window,
                        event.atom,
                        atoms.TextUriList,
                        &bytes[transfer.offset..end],
                    )?
                    .check()?;
                if transfer.offset == bytes.len() {
                    drag.transfers.remove(index);
                } else {
                    transfer.offset = end;
                    transfer.deadline = Instant::now() + Duration::from_secs(10);
                }
                connection.flush()?;
                Ok(true)
            }
            Event::KeyRelease(_)
            | Event::ButtonRelease(_)
            | Event::MotionNotify(_)
            | Event::XinputButtonRelease(_)
            | Event::XinputMotion(_) => Ok(!drag.released),
            _ => Ok(false),
        }
    }
}
