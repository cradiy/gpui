use std::{
    cell::{RefCell, RefMut},
    hash::Hash,
    os::fd::{AsRawFd, BorrowedFd},
    path::PathBuf,
    rc::{Rc, Weak},
    sync::Mutex,
    time::{Duration, Instant},
};

use anyhow::Context as _;
use ashpd::WindowIdentifier;
use calloop::{
    EventLoop, LoopHandle,
    timer::{TimeoutAction, Timer},
};
use collections::HashMap;
use filedescriptor::Pipe;
use gpui_util::ResultExt as _;
use gpui_wayland_source::insert_wayland_source;
use http_client::Url;
use smallvec::SmallVec;
use wayland_backend::client::ObjectId;
use wayland_backend::protocol::WEnum;
use wayland_client::event_created_child;
use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_data_device_manager::DndAction;
use wayland_client::protocol::wl_data_offer::WlDataOffer;
use wayland_client::protocol::wl_pointer::AxisSource;
use wayland_client::protocol::{
    wl_data_device, wl_data_device_manager, wl_data_offer, wl_data_source, wl_output, wl_region,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_surface,
    },
};
use wayland_protocols::wp::pointer_gestures::zv1::client::{
    zwp_pointer_gesture_pinch_v1, zwp_pointer_gestures_v1,
};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::{
    self, ZwpPrimarySelectionOfferV1,
};
use wayland_protocols::wp::primary_selection::zv1::client::{
    zwp_primary_selection_device_manager_v1, zwp_primary_selection_device_v1,
    zwp_primary_selection_source_v1,
};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::{
    ChangeCause, ContentHint, ContentPurpose,
};
use wayland_protocols::wp::text_input::zv3::client::{
    zwp_text_input_manager_v3, zwp_text_input_v3,
};
use wayland_protocols::wp::viewporter::client::{wp_viewport, wp_viewporter};
use wayland_protocols::xdg::activation::v1::client::{xdg_activation_token_v1, xdg_activation_v1};
use wayland_protocols::xdg::decoration::zv1::client::{
    zxdg_decoration_manager_v1, zxdg_toplevel_decoration_v1,
};
use wayland_protocols::xdg::shell::client::{
    xdg_popup, xdg_positioner, xdg_surface, xdg_toplevel, xdg_wm_base,
};
use wayland_protocols::xdg::system_bell::v1::client::xdg_system_bell_v1;
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols::{
    wp::cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1},
    xdg::dialog::v1::client::xdg_wm_dialog_v1::{self, XdgWmDialogV1},
};
use wayland_protocols::{
    wp::fractional_scale::v1::client::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    xdg::dialog::v1::client::xdg_dialog_v1::XdgDialogV1,
};
use wayland_protocols_plasma::blur::client::{org_kde_kwin_blur, org_kde_kwin_blur_manager};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};
use xkbcommon::xkb;

use super::{
    display::WaylandDisplay,
    keyboard::{load_keymap, translate_key, update_modifiers},
    text_input::{InputContext, MAX_SURROUNDING_BYTES, Preedit, TextInputState},
    window::{ImeInput, WaylandDragIcon, WaylandWindowStatePtr},
};

use crate::linux::{
    DOUBLE_CLICK_INTERVAL, LinuxClient, LinuxCommon, LinuxKeyboardLayout, PIPE_READ_TIMEOUT,
    SCROLL_LINES, cursor_style_to_icon_names, dispatch_tray_message, get_xkb_compose_state,
    is_within_click_distance, keystroke_underlying_dead_key, open_uri_internal,
    read_fd_with_timeout, reveal_path_internal,
    wayland::{
        clipboard::{Clipboard, DataOffer, FILE_LIST_MIME_TYPE},
        cursor::Cursor,
        external_surface::ExternalWaylandSurfaceRoleFactory,
        serial::{SerialKind, SerialTracker},
        to_shape,
        window::WaylandWindow,
    },
    xdg_desktop_portal::{Event as XDPEvent, XDPEventSource},
};
use gpui::{
    AnyWindowHandle, Bounds, Capslock, CursorStyle, DevicePixels, DisplayId, DragSessionId,
    FileDropEvent, ForegroundExecutor, InternalDragEvent, KeyDownEvent, KeyUpEvent, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseExitEvent, MouseMoveEvent,
    MouseUpEvent, NavigationDirection, Pixels, PlatformDisplay, PlatformInput,
    PlatformKeyboardLayout, PlatformWindow, Point, ScrollDelta, ScrollWheelEvent, SharedString,
    Size, TouchPhase, WindowButtonLayout, WindowKind, WindowParams, point, profiler, px, size,
};
use gpui_wgpu::{CompositorGpuHint, GpuContext};
use wayland_protocols::wp::linux_dmabuf::zv1::client::{
    zwp_linux_dmabuf_feedback_v1, zwp_linux_dmabuf_v1,
};

const UNKNOWN_KEYBOARD_LAYOUT_NAME: SharedString = SharedString::new_static("unknown");
const XDG_ACTIVATION_TOKEN_ENV_VAR: &str = "XDG_ACTIVATION_TOKEN";

fn take_startup_activation_token_from_environment() -> Option<String> {
    let startup_activation_token = std::env::var(XDG_ACTIVATION_TOKEN_ENV_VAR)
        .ok()
        .filter(|token| !token.is_empty());
    // The token must be removed from the environment so it isn't inherited by child
    // processes we spawn, per the xdg-activation spec: https://wayland.app/protocols/xdg-activation-v1
    // SAFETY: This runs during Wayland platform initialization before GPUI starts
    // concurrent environment access or spawning child processes.
    unsafe { std::env::remove_var(XDG_ACTIVATION_TOKEN_ENV_VAR) };
    startup_activation_token
}

#[derive(Clone)]
pub struct Globals {
    pub qh: QueueHandle<WaylandClientStatePtr>,
    pub activation: Option<xdg_activation_v1::XdgActivationV1>,
    pub compositor: wl_compositor::WlCompositor,
    pub cursor_shape_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    pub data_device_manager: Option<wl_data_device_manager::WlDataDeviceManager>,
    pub primary_selection_manager:
        Option<zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1>,
    pub wm_base: xdg_wm_base::XdgWmBase,
    pub shm: wl_shm::WlShm,
    pub seat: wl_seat::WlSeat,
    pub viewporter: Option<wp_viewporter::WpViewporter>,
    pub fractional_scale_manager:
        Option<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1>,
    pub decoration_manager: Option<zxdg_decoration_manager_v1::ZxdgDecorationManagerV1>,
    pub layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    pub blur_manager: Option<org_kde_kwin_blur_manager::OrgKdeKwinBlurManager>,
    pub text_input_manager: Option<zwp_text_input_manager_v3::ZwpTextInputManagerV3>,
    pub gesture_manager: Option<zwp_pointer_gestures_v1::ZwpPointerGesturesV1>,
    pub dialog: Option<xdg_wm_dialog_v1::XdgWmDialogV1>,
    pub system_bell: Option<xdg_system_bell_v1::XdgSystemBellV1>,
    pub xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    pub executor: ForegroundExecutor,
}

impl Globals {
    fn new(
        globals: GlobalList,
        executor: ForegroundExecutor,
        qh: QueueHandle<WaylandClientStatePtr>,
        seat: wl_seat::WlSeat,
    ) -> Self {
        let dialog_v = XdgWmDialogV1::interface().version;
        Globals {
            activation: globals.bind(&qh, 1..=1, ()).ok(),
            compositor: globals
                .bind(
                    &qh,
                    wl_surface::REQ_SET_BUFFER_SCALE_SINCE
                        ..=wl_surface::EVT_PREFERRED_BUFFER_SCALE_SINCE,
                    (),
                )
                .unwrap(),
            cursor_shape_manager: globals.bind(&qh, 1..=1, ()).ok(),
            data_device_manager: globals
                .bind(
                    &qh,
                    WL_DATA_DEVICE_MANAGER_VERSION..=WL_DATA_DEVICE_MANAGER_VERSION,
                    (),
                )
                .ok(),
            primary_selection_manager: globals.bind(&qh, 1..=1, ()).ok(),
            shm: globals.bind(&qh, 1..=1, ()).unwrap(),
            seat,
            wm_base: globals.bind(&qh, 1..=5, ()).unwrap(),
            viewporter: globals.bind(&qh, 1..=1, ()).ok(),
            fractional_scale_manager: globals.bind(&qh, 1..=1, ()).ok(),
            decoration_manager: globals.bind(&qh, 1..=1, ()).ok(),
            layer_shell: globals.bind(&qh, 1..=5, ()).ok(),
            blur_manager: globals.bind(&qh, 1..=1, ()).ok(),
            text_input_manager: globals.bind(&qh, 1..=1, ()).ok(),
            gesture_manager: globals.bind(&qh, 1..=3, ()).ok(),
            dialog: globals.bind(&qh, dialog_v..=dialog_v, ()).ok(),
            system_bell: globals.bind(&qh, 1..=1, ()).ok(),
            xdg_output_manager: globals.bind(&qh, 1..=3, ()).ok(),
            executor,
            qh,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct InProgressOutput {
    name: Option<String>,
    scale: Option<i32>,
    position: Option<Point<DevicePixels>>,
    size: Option<Size<DevicePixels>>,
    subpixel: Option<wl_output::Subpixel>,
    transform: Option<wl_output::Transform>,
    xdg_version: Option<u32>,
    logical_position: Option<Point<i32>>,
    logical_size: Option<Size<i32>>,
    logical_bounds: Option<Bounds<i32>>,
}

impl InProgressOutput {
    // Keep the last values between Done events: subsequent batches contain
    // only the properties that changed.
    fn apply(&mut self, event: wl_output::Event) -> Option<Output> {
        match event {
            wl_output::Event::Name { name } => self.name = Some(name),
            wl_output::Event::Scale { factor } if factor > 0 => self.scale = Some(factor),
            wl_output::Event::Geometry {
                x,
                y,
                subpixel,
                transform,
                ..
            } => {
                self.position = Some(point(DevicePixels(x), DevicePixels(y)));
                self.subpixel = match subpixel {
                    WEnum::Value(value) => Some(value),
                    _ => None,
                };
                self.transform = match transform {
                    WEnum::Value(value) => Some(value),
                    _ => None,
                };
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(flags),
                width,
                height,
                ..
            } if flags.contains(wl_output::Mode::Current) && width > 0 && height > 0 => {
                self.size = Some(size(DevicePixels(width), DevicePixels(height)));
            }
            wl_output::Event::Done => {
                if self.xdg_version.is_some_and(|version| version >= 3) {
                    self.commit_logical_bounds();
                }
                return self.complete();
            }
            _ => {}
        }
        None
    }

    fn apply_xdg(&mut self, event: zxdg_output_v1::Event) -> Option<Output> {
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                self.logical_position = Some(point(x, y));
            }
            zxdg_output_v1::Event::LogicalSize { width, height } if width > 0 && height > 0 => {
                self.logical_size = Some(size(width, height));
            }
            zxdg_output_v1::Event::Name { name } if self.name.is_none() => {
                self.name = Some(name);
            }
            zxdg_output_v1::Event::Done if self.xdg_version.is_some_and(|version| version < 3) => {
                self.commit_logical_bounds();
                return self.complete();
            }
            _ => {}
        }
        None
    }

    fn commit_logical_bounds(&mut self) {
        if let Some((origin, size)) = self.logical_position.zip(self.logical_size) {
            self.logical_bounds = Some(Bounds::new(origin, size));
        }
    }

    fn complete(&self) -> Option<Output> {
        // Do not expose integer-scale estimates while the initial xdg batch is pending.
        if self.xdg_version.is_some() && self.logical_bounds.is_none() {
            return None;
        }
        if let Some((position, mut size)) = self.position.zip(self.size) {
            if matches!(
                self.transform,
                Some(
                    wl_output::Transform::_90
                        | wl_output::Transform::_270
                        | wl_output::Transform::Flipped90
                        | wl_output::Transform::Flipped270
                )
            ) {
                std::mem::swap(&mut size.width, &mut size.height);
            }
            let scale = self.scale.unwrap_or(1);
            Some(Output {
                name: self.name.clone(),
                scale,
                bounds: Bounds::new(position, size),
                logical_bounds: self.logical_bounds,
                subpixel: self.subpixel,
            })
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct Output {
    pub name: Option<String>,
    pub scale: i32,
    pub bounds: Bounds<DevicePixels>,
    pub logical_bounds: Option<Bounds<i32>>,
    pub subpixel: Option<wl_output::Subpixel>,
}

impl Output {
    pub fn logical_bounds(&self) -> Bounds<Pixels> {
        if let Some(bounds) = self.logical_bounds {
            return Bounds::new(
                point(px(bounds.origin.x as f32), px(bounds.origin.y as f32)),
                size(px(bounds.size.width as f32), px(bounds.size.height as f32)),
            );
        }
        // wl_output.geometry is already in compositor space. Only the mode size
        // is in device pixels and needs conversion when xdg-output is unavailable.
        Bounds::new(
            point(
                px(self.bounds.origin.x.0 as f32),
                px(self.bounds.origin.y.0 as f32),
            ),
            self.bounds.size.to_pixels(self.scale as f32),
        )
    }

    pub fn display_scale_factor(&self) -> f32 {
        // xdg-output has no explicit fractional scale; this is an estimate from
        // the rounded logical size. A window's preferred scale stays authoritative.
        self.logical_bounds.map_or(self.scale as f32, |bounds| {
            self.bounds.size.width.0 as f32 / bounds.size.width as f32
        })
    }
}

pub(crate) struct WaylandClientState {
    serial_tracker: SerialTracker,
    globals: Globals,
    external_surface_role: Option<ExternalWaylandSurfaceRoleFactory>,
    pub gpu_context: GpuContext,
    pub compositor_gpu: Option<CompositorGpuHint>,
    wl_seat: wl_seat::WlSeat, // TODO: Multi seat support
    wl_pointer: Option<wl_pointer::WlPointer>,
    pinch_gesture: Option<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1>,
    pinch_scale: f32,
    wl_keyboard: Option<wl_keyboard::WlKeyboard>,
    cursor_shape_device: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    data_device: Option<wl_data_device::WlDataDevice>,
    internal_drag_mime: String,
    native_drag_source: Option<NativeDragSource>,
    pending_drag_icons: HashMap<DragSessionId, WaylandDragIcon>,
    primary_selection: Option<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1>,
    text_input: Option<zwp_text_input_v3::ZwpTextInputV3>,
    pre_edit_text: Option<String>,
    ime: TextInputState,
    text_input_surface: Option<ObjectId>,
    composing: bool,
    // Surface to Window mapping
    windows: HashMap<ObjectId, WaylandWindowStatePtr>,
    // Output to scale mapping
    outputs: HashMap<ObjectId, Output>,
    in_progress_outputs: HashMap<ObjectId, InProgressOutput>,
    wl_outputs: HashMap<ObjectId, wl_output::WlOutput>,
    xdg_outputs: HashMap<ObjectId, zxdg_output_v1::ZxdgOutputV1>,
    output_globals: HashMap<u32, ObjectId>,
    keyboard_layout: LinuxKeyboardLayout,
    keymap_state: Option<xkb::State>,
    compose_state: Option<xkb::compose::State>,
    drag: DragState,
    click: ClickState,
    repeat: KeyRepeat,
    pub modifiers: Modifiers,
    pub capslock: Capslock,
    axis_source: AxisSource,
    pub mouse_location: Option<Point<Pixels>>,
    continuous_scroll_delta: Option<Point<Pixels>>,
    discrete_scroll_delta: Option<Point<f32>>,
    vertical_modifier: f32,
    horizontal_modifier: f32,
    scroll_event_received: bool,
    enter_token: Option<()>,
    button_pressed: Option<MouseButton>,
    mouse_focused_window: Option<WaylandWindowStatePtr>,
    keyboard_focused_window: Option<WaylandWindowStatePtr>,
    activation_history: crate::linux::platform::ActivationHistory<ObjectId>,
    loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    cursor_style: Option<CursorStyle>,
    cursor_hidden_window: Option<WaylandWindowStatePtr>,
    clipboard: Clipboard,
    data_offers: Vec<DataOffer<WlDataOffer>>,
    primary_data_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>,
    cursor: Cursor,
    startup_activation_token: Option<String>,
    event_loop: Option<EventLoop<'static, WaylandClientStatePtr>>,
    pub common: LinuxCommon,
    ime_enabled: Option<bool>,
}

pub struct DragState {
    data_offer: Option<wl_data_offer::WlDataOffer>,
    kind: Option<DragOfferKind>,
    window: Option<WaylandWindowStatePtr>,
    position: Point<Pixels>,
}

#[derive(Default)]
struct WaylandDataOfferState {
    selected_action: Mutex<Option<DndAction>>,
}

impl WaylandDataOfferState {
    fn selected_action(&self) -> Option<DndAction> {
        *self
            .selected_action
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn set_selected_action(&self, action: Option<DndAction>) {
        *self
            .selected_action
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = action;
    }
}

fn data_offer_can_finish(accepted: bool, selected_action: Option<DndAction>) -> bool {
    accepted && selected_action.is_some_and(|action| !action.is_empty())
}

fn finish_data_offer(data_offer: wl_data_offer::WlDataOffer, accepted: bool) {
    let selected_action = data_offer
        .data::<WaylandDataOfferState>()
        .and_then(WaylandDataOfferState::selected_action);
    if data_offer_can_finish(accepted, selected_action) {
        data_offer.finish();
    } else if accepted {
        log::warn!(
            "[gpui-dnd] refusing wl_data_offer.finish without a valid action offer={:?}",
            data_offer.id()
        );
    }
    data_offer.destroy();
}

#[derive(Clone, Copy)]
enum DragOfferKind {
    Internal(DragSessionId),
    ExternalFiles,
}

struct NativeDragSource {
    session_id: DragSessionId,
    window: WaylandWindowStatePtr,
    data_source: wl_data_source::WlDataSource,
    icon: Option<WaylandDragIcon>,
    files: Option<gpui::SystemFileDrag>,
    action: Option<gpui::DragAction>,
    external_target: bool,
    drop_performed: bool,
    transfers: DragTransfers,
}

struct DragTransfers {
    handle: LoopHandle<'static, WaylandClientStatePtr>,
    tokens: Vec<calloop::RegistrationToken>,
}
impl Drop for DragTransfers {
    fn drop(&mut self) {
        for token in self.tokens.drain(..) {
            self.handle.remove(token);
        }
    }
}

fn file_actions(files: Option<&gpui::SystemFileDrag>) -> DndAction {
    let Some(files) = files else {
        return DndAction::Move;
    };
    let actions = files.options().allowed_actions;
    let mut result = DndAction::empty();
    if actions.contains(gpui::DragActions::COPY) {
        result |= DndAction::Copy;
    }
    if actions.contains(gpui::DragActions::MOVE) {
        result |= DndAction::Move;
    }
    result
}

#[derive(Clone)]
enum WaylandDataSourceKind {
    Clipboard(crate::linux::clipboard::ClipboardSource),
    InternalDrag(DragSessionId),
}

pub struct ClickState {
    last_mouse_button: Option<MouseButton>,
    last_click: Instant,
    last_location: Point<Pixels>,
    current_count: usize,
}

pub(crate) struct KeyRepeat {
    characters_per_second: u32,
    delay: Duration,
    current_id: u64,
    current_keycode: Option<xkb::Keycode>,
}

pub(crate) enum PendingActivation {
    /// URI to open in the web browser.
    Uri(String),
    /// Path to open in the file explorer.
    Path(PathBuf),
    /// A window from ourselves to raise.
    Window(ObjectId),
}

impl WaylandClientState {
    fn publish_surrounding(
        &mut self,
        text_input: &zwp_text_input_v3::ZwpTextInputV3,
        mut context: InputContext,
        cause: ChangeCause,
    ) -> bool {
        context.surrounding = context.surrounding.filter(|text| {
            text.text.len() <= MAX_SURROUNDING_BYTES
                && !text.text.contains('\0')
                && text.text.is_char_boundary(text.cursor)
                && text.text.is_char_boundary(text.anchor)
        });
        if self.ime.context.as_ref() == Some(&context) {
            return false;
        }
        let reset = self.ime.context.as_ref().is_some_and(|old| {
            old.focus != context.focus || old.surrounding.is_some() != context.surrounding.is_some()
        });
        if reset {
            text_input.disable();
            text_input.commit();
            self.ime.committed();
            text_input.enable();
            text_input.set_content_type(ContentHint::None, ContentPurpose::Normal);
            self.ime.reset_context();
            self.composing = false;
        }
        text_input.set_text_change_cause(cause);
        if let Some(surrounding) = &context.surrounding {
            text_input.set_surrounding_text(
                surrounding.text.clone(),
                surrounding.cursor as i32,
                surrounding.anchor as i32,
            );
        }
        self.ime.record_context(context);
        true
    }

    fn consume_startup_activation_token(&mut self, surface: &wl_surface::WlSurface) {
        let Some(startup_activation_token) = self.startup_activation_token.take() else {
            return;
        };
        let Some(activation) = self.globals.activation.as_ref() else {
            return;
        };
        activation.activate(startup_activation_token, surface);
    }
}

/// This struct is required to conform to Rust's orphan rules, so we can dispatch on the state but hand the
/// window to GPUI.
#[derive(Clone)]
pub struct WaylandClientStatePtr(Weak<RefCell<WaylandClientState>>);

impl WaylandClientStatePtr {
    fn publish_output(&self, id: &ObjectId, output: Output) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        if state.outputs.get(id) == Some(&output) {
            return;
        }
        state.outputs.insert(id.clone(), output.clone());
        let windows = state.windows.values().cloned().collect::<Vec<_>>();
        drop(state);
        for window in windows {
            window.update_output(id, &output);
        }
    }

    pub(super) fn output_for_display(&self, display_id: DisplayId) -> Option<wl_output::WlOutput> {
        self.get_client().borrow().output_for_display(display_id)
    }

    fn fail_drag(&self, session_id: DragSessionId, failure: gpui::DragFailure) {
        let window = self
            .get_client()
            .borrow()
            .native_drag_source
            .as_ref()
            .filter(|source| source.session_id == session_id)
            .map(|source| source.window.clone());
        self.cancel_internal_drag(session_id);
        if let Some(window) = window {
            window.handle_input(PlatformInput::InternalDrag(
                InternalDragEvent::SourceFailed {
                    session_id,
                    failure,
                },
            ));
        }
    }

    fn drag_timeout(&self, session_id: DragSessionId) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let handle = state.loop_handle.clone();
        if let Ok(token) = handle.insert_source(
            Timer::from_duration(Duration::from_secs(60)),
            move |_, _, client| {
                client.fail_drag(session_id, gpui::DragFailure::TimedOut);
                TimeoutAction::Drop
            },
        ) {
            if let Some(source) = state
                .native_drag_source
                .as_mut()
                .filter(|source| source.session_id == session_id)
            {
                source.transfers.tokens.push(token);
            } else {
                handle.remove(token);
            }
        }
    }

    fn send_drag_files(
        &self,
        session_id: DragSessionId,
        fd: std::os::fd::OwnedFd,
        bytes: std::sync::Arc<[u8]>,
    ) -> anyhow::Result<()> {
        use std::io::{ErrorKind, Write};
        // A writable pipe may have less room than the payload. Never block the UI loop.
        let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
        anyhow::ensure!(
            flags >= 0
                && unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                    >= 0,
            "cannot make file drag pipe nonblocking"
        );
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let handle = state.loop_handle.clone();
        let completed = Rc::new(std::cell::Cell::new(false));
        let done = completed.clone();
        let mut offset = 0;
        let token =
            handle
                .insert_source(
                    calloop::generic::Generic::new(
                        std::fs::File::from(fd),
                        calloop::Interest::WRITE,
                        calloop::Mode::Level,
                    ),
                    move |_, file, client| {
                        let file = unsafe { file.get_mut() };
                        // Bound work per event so a large file selection does not monopolize rendering.
                        let end = (offset + 65536).min(bytes.len());
                        match file.write(&bytes[offset..end]) {
                            Ok(n) if n > 0 => {
                                offset += n;
                                if offset == bytes.len() {
                                    done.set(true);
                                    return Ok(calloop::PostAction::Remove);
                                }
                            }
                            Err(error)
                                if matches!(
                                    error.kind(),
                                    ErrorKind::WouldBlock | ErrorKind::Interrupted
                                ) => {}
                            _ => {
                                done.set(true);
                                client.get_client().borrow().loop_handle.insert_idle(
                                    move |client| {
                                        client.fail_drag(session_id, gpui::DragFailure::Transfer)
                                    },
                                );
                                return Ok(calloop::PostAction::Remove);
                            }
                        }
                        Ok(calloop::PostAction::Continue)
                    },
                )
                .map_err(|error| anyhow::anyhow!("cannot register file drag pipe: {error}"))?;
        let timer = match handle.insert_source(
            Timer::from_duration(Duration::from_secs(10)),
            move |_, _, client| {
                if !completed.get() {
                    client.fail_drag(session_id, gpui::DragFailure::TimedOut);
                }
                TimeoutAction::Drop
            },
        ) {
            Ok(timer) => timer,
            Err(error) => {
                handle.remove(token);
                return Err(anyhow::anyhow!(
                    "cannot register file drag timeout: {error}"
                ));
            }
        };
        if let Some(source) = state
            .native_drag_source
            .as_mut()
            .filter(|source| source.session_id == session_id)
        {
            source.transfers.tokens.extend([token, timer]);
        } else {
            handle.remove(token);
            handle.remove(timer);
        }
        Ok(())
    }

    pub fn get_client(&self) -> Rc<RefCell<WaylandClientState>> {
        self.0
            .upgrade()
            .expect("The pointer should always be valid when dispatching in wayland")
    }

    pub fn get_serial(&self, kind: SerialKind) -> u32 {
        self.0.upgrade().unwrap().borrow().serial_tracker.get(kind)
    }

    pub fn start_internal_drag(
        &self,
        source_window: WaylandWindowStatePtr,
        session_id: DragSessionId,
        has_icon: bool,
        files: Option<gpui::SystemFileDrag>,
    ) -> anyhow::Result<()> {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let manager = state
            .globals
            .data_device_manager
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Wayland data-device manager is unavailable"))?;
        let data_device = state
            .data_device
            .clone()
            .ok_or_else(|| anyhow::anyhow!("Wayland data device is unavailable"))?;
        let serial = state.serial_tracker.get(SerialKind::MousePress);
        if serial == 0 {
            anyhow::bail!("Wayland has no initiating mouse-press serial for this drag");
        }
        if state.native_drag_source.is_some() {
            anyhow::bail!("another Wayland native drag is already active");
        }

        let mut icon = if has_icon {
            Some(
                state
                    .pending_drag_icons
                    .remove(&session_id)
                    .ok_or_else(|| anyhow::anyhow!("drag icon was not created for this session"))?,
            )
        } else {
            None
        };

        let data_source = manager.create_data_source(
            &state.globals.qh,
            WaylandDataSourceKind::InternalDrag(session_id),
        );
        data_source.offer(state.internal_drag_mime.clone());
        if files.is_some() {
            data_source.offer(FILE_LIST_MIME_TYPE.into());
        }
        data_source.set_actions(file_actions(files.as_ref()));
        data_device.start_drag(
            Some(&data_source),
            &source_window.surface(),
            icon.as_ref().map(WaylandDragIcon::surface),
            serial,
        );
        if let Some(icon) = icon.as_mut() {
            icon.activate_role();
        }
        if let Err(error) = source_window
            .surface()
            .backend()
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("Wayland connection is unavailable"))?
            .flush()
            .context("failed to flush the Wayland drag request")
        {
            data_source.destroy();
            return Err(error);
        }
        state.native_drag_source = Some(NativeDragSource {
            session_id,
            window: source_window,
            data_source,
            icon,
            files,
            action: None,
            external_target: false,
            drop_performed: false,
            transfers: DragTransfers {
                handle: state.loop_handle.clone(),
                tokens: Vec::new(),
            },
        });
        Ok(())
    }

    pub fn create_internal_drag_icon(
        &self,
        source_window: &WaylandWindowStatePtr,
        session_id: DragSessionId,
        logical_size: Size<Pixels>,
        scale_factor: f32,
        hotspot: Point<Pixels>,
        scene: &gpui::Scene,
    ) -> anyhow::Result<()> {
        let icon = source_window.create_drag_icon(logical_size, scale_factor, hotspot, scene)?;
        let client = self.get_client();
        let mut state = client.borrow_mut();
        if let Some(mut old_icon) = state.pending_drag_icons.insert(session_id, icon) {
            old_icon.destroy_once();
        }
        Ok(())
    }

    pub fn update_internal_drag_icon(
        &self,
        session_id: DragSessionId,
        logical_size: Size<Pixels>,
        scale_factor: f32,
        hotspot: Point<Pixels>,
        scene: &gpui::Scene,
    ) -> anyhow::Result<()> {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        if let Some(icon) = state
            .native_drag_source
            .as_mut()
            .filter(|source| source.session_id == session_id)
            .and_then(|source| source.icon.as_mut())
        {
            return icon.draw(logical_size, scale_factor, hotspot, scene);
        }
        if let Some(icon) = state.pending_drag_icons.get_mut(&session_id) {
            return icon.draw(logical_size, scale_factor, hotspot, scene);
        }
        anyhow::bail!("drag icon is not active for this session")
    }

    pub fn destroy_internal_drag_icon(&self, session_id: DragSessionId) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        if let Some(mut icon) = state.pending_drag_icons.remove(&session_id) {
            icon.destroy_once();
        }
        if let Some(icon) = state
            .native_drag_source
            .as_mut()
            .filter(|source| source.session_id == session_id)
            .and_then(|source| source.icon.as_mut())
        {
            icon.destroy_once();
        }
    }

    pub fn cancel_internal_drag(&self, session_id: DragSessionId) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let Some(mut source) = state.native_drag_source.take() else {
            return;
        };
        if source.session_id == session_id {
            drop(source.icon.take());
            source.data_source.destroy();
        } else {
            state.native_drag_source = Some(source);
        }
    }

    pub(crate) fn activation_context(&self) -> (u32, Option<wl_surface::WlSurface>) {
        let client = self.get_client();
        let state = client.borrow();
        let serial = state.serial_tracker.get_latest_input();
        let source = if serial == state.serial_tracker.get(SerialKind::KeyPress) {
            state.keyboard_focused_window.as_ref()
        } else {
            state.mouse_focused_window.as_ref()
        };
        (serial, source.map(WaylandWindowStatePtr::surface))
    }

    pub fn enable_ime(&self) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        state.ime_enabled = Some(true);
        state.ime.reset_context();
        let Some(text_input) = state.text_input.take() else {
            return;
        };

        text_input.enable();
        text_input.set_content_type(ContentHint::None, ContentPurpose::Normal);
        let window = state
            .text_input_surface
            .as_ref()
            .and_then(|surface| state.windows.get(surface))
            .cloned();
        if let Some(window) = window {
            drop(state);
            let area = window.get_ime_area();
            let context = window.ime_context();
            state = client.borrow_mut();
            state.publish_surrounding(&text_input, context, ChangeCause::Other);
            if let Some([x, y, width, height]) =
                area.and_then(|area| state.ime.update_cursor_rectangle(area))
            {
                text_input.set_cursor_rectangle(x, y, width, height);
            }
        }
        text_input.commit();
        state.ime.committed();
        state.text_input = Some(text_input);
    }

    pub fn disable_ime(&self) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        state.ime_enabled = Some(false);
        state.composing = false;
        state.ime.reset_context();
        if let Some(text_input) = &state.text_input {
            text_input.disable();
            text_input.commit();
            state.ime.committed();
        }
    }

    pub fn ime_enabled(&self) -> Option<bool> {
        let client = self.get_client();
        client.borrow().ime_enabled
    }

    pub(super) fn update_surrounding_text(
        &self,
        surface: &ObjectId,
        context: InputContext,
        area: Option<Bounds<Pixels>>,
    ) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        if state.ime_enabled != Some(true)
            || state.text_input_surface.as_ref() != Some(surface)
            || state.ime.defer_publish
        {
            return;
        }
        let Some(text_input) = state.text_input.clone() else {
            return;
        };
        let mut changed = state.publish_surrounding(&text_input, context, ChangeCause::Other);
        if let Some([x, y, width, height]) =
            area.and_then(|area| state.ime.update_cursor_rectangle(area))
        {
            text_input.set_cursor_rectangle(x, y, width, height);
            changed = true;
        }
        if changed {
            text_input.commit();
            state.ime.committed();
        }
    }

    pub fn update_ime_position(&self, bounds: Bounds<Pixels>) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        if state.text_input.is_none() || state.pre_edit_text.is_some() || state.ime.defer_publish {
            return;
        }

        let Some([x, y, width, height]) = state.ime.update_cursor_rectangle(bounds) else {
            return;
        };
        let text_input = state.text_input.as_ref().unwrap();
        text_input.set_cursor_rectangle(x, y, width, height);
        text_input.commit();
        state.ime.committed();
    }

    pub fn handle_keyboard_layout_change(&self) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let changed = if let Some(keymap_state) = &state.keymap_state {
            let layout_idx = keymap_state.serialize_layout(xkbcommon::xkb::STATE_LAYOUT_EFFECTIVE);
            let keymap = keymap_state.get_keymap();
            let layout_name = keymap.layout_get_name(layout_idx);
            let changed = layout_name != state.keyboard_layout.name();
            if changed {
                state.keyboard_layout = LinuxKeyboardLayout::new(layout_name.to_string().into());
            }
            changed
        } else {
            let changed = &UNKNOWN_KEYBOARD_LAYOUT_NAME != state.keyboard_layout.name();
            if changed {
                state.keyboard_layout = LinuxKeyboardLayout::new(UNKNOWN_KEYBOARD_LAYOUT_NAME);
            }
            changed
        };

        if changed && let Some(mut callback) = state.common.callbacks.keyboard_layout_change.take()
        {
            drop(state);
            callback();
            state = client.borrow_mut();
            state.common.callbacks.keyboard_layout_change = Some(callback);
        }
    }

    pub fn drop_window(&self, surface_id: &ObjectId) {
        let client = self.get_client();
        let mut state = client.borrow_mut();
        let closed_window = state.windows.remove(surface_id).unwrap();
        state.activation_history.closed(surface_id);
        if state
            .native_drag_source
            .as_ref()
            .is_some_and(|source| source.window.ptr_eq(&closed_window))
        {
            if let Some(source) = state.native_drag_source.take() {
                source.data_source.destroy();
            }
        }
        if let Some(window) = state.mouse_focused_window.take()
            && !window.ptr_eq(&closed_window)
        {
            state.mouse_focused_window = Some(window);
        }
        if let Some(window) = state.keyboard_focused_window.take()
            && !window.ptr_eq(&closed_window)
        {
            state.keyboard_focused_window = Some(window);
        }
        if let Some(window) = state.cursor_hidden_window.take()
            && !window.ptr_eq(&closed_window)
        {
            state.cursor_hidden_window = Some(window);
        }
    }
}

impl WaylandClientState {
    fn output_for_display(&self, display_id: DisplayId) -> Option<wl_output::WlOutput> {
        let protocol_id: u64 = display_id.into();
        self.wl_outputs
            .iter()
            .find(|(id, _)| id.protocol_id() as u64 == protocol_id)
            .map(|(_, output)| output.clone())
    }

    fn hide_cursor_until_mouse_moves(&mut self) {
        if self.cursor_hidden_window.is_some() {
            return;
        }
        let Some(focused_window) = self.mouse_focused_window.clone() else {
            // No surface to apply the hidden cursor to.
            return;
        };
        let Some(wl_pointer) = self.wl_pointer.clone() else {
            // Seat lost its pointer capability; nothing to hide.
            return;
        };
        let serial = self.serial_tracker.get(SerialKind::MouseEnter);
        wl_pointer.set_cursor(serial, None, 0, 0);
        self.cursor_hidden_window = Some(focused_window);
    }

    fn restore_cursor_after_hide(&mut self) {
        if self.cursor_hidden_window.take().is_none() {
            return;
        }
        let Some(style) = self.cursor_style else {
            return;
        };
        let serial = self.serial_tracker.get(SerialKind::MouseEnter);
        if let Some(cursor_shape_device) = &self.cursor_shape_device {
            cursor_shape_device.set_shape(serial, to_shape(style));
            return;
        }
        let Some(focused_window) = self.mouse_focused_window.clone() else {
            log::warn!(
                "wayland: no focused surface to restore cursor style {:?} after hide; cursor may stay invisible",
                style
            );
            return;
        };
        let Some(wl_pointer) = self.wl_pointer.clone() else {
            log::warn!(
                "wayland: no wl_pointer to restore cursor style {:?} after hide; cursor may stay invisible",
                style
            );
            return;
        };
        let scale = focused_window.primary_output_scale();
        self.cursor.set_icon(
            &wl_pointer,
            serial,
            cursor_style_to_icon_names(style),
            scale,
        );
    }
}

#[derive(Clone)]
pub struct WaylandClient(Rc<RefCell<WaylandClientState>>);

impl Drop for WaylandClient {
    fn drop(&mut self) {
        // Temporary client clones share all windows and input objects. Only the
        // final owner may tear down the connection state.
        if Rc::strong_count(&self.0) != 1 {
            return;
        }
        let mut state = self.0.borrow_mut();
        state.windows.clear();
        for (_, output) in state.xdg_outputs.drain() {
            output.destroy();
        }
        if let Some(manager) = &state.globals.xdg_output_manager {
            manager.destroy();
        }

        if let Some(wl_pointer) = &state.wl_pointer {
            wl_pointer.release();
        }
        if let Some(cursor_shape_device) = &state.cursor_shape_device {
            cursor_shape_device.destroy();
        }
        if let Some(data_device) = &state.data_device {
            data_device.release();
        }
        if let Some(text_input) = &state.text_input {
            text_input.destroy();
        }
    }
}

const WL_DATA_DEVICE_MANAGER_VERSION: u32 = 3;

fn wl_seat_version(version: u32) -> u32 {
    // We rely on the wl_pointer.frame event
    const WL_SEAT_MIN_VERSION: u32 = 5;
    const WL_SEAT_MAX_VERSION: u32 = 9;

    if version < WL_SEAT_MIN_VERSION {
        panic!(
            "wl_seat below required version: {} < {}",
            version, WL_SEAT_MIN_VERSION
        );
    }

    version.clamp(WL_SEAT_MIN_VERSION, WL_SEAT_MAX_VERSION)
}

fn wl_output_version(version: u32) -> u32 {
    const WL_OUTPUT_MIN_VERSION: u32 = 2;
    const WL_OUTPUT_MAX_VERSION: u32 = 4;

    if version < WL_OUTPUT_MIN_VERSION {
        panic!(
            "wl_output below required version: {} < {}",
            version, WL_OUTPUT_MIN_VERSION
        );
    }

    version.clamp(WL_OUTPUT_MIN_VERSION, WL_OUTPUT_MAX_VERSION)
}

impl WaylandClient {
    pub(crate) fn new() -> Self {
        let startup_activation_token = take_startup_activation_token_from_environment();
        let conn = Connection::connect_to_env().unwrap();
        Self::with_connection(conn, startup_activation_token, None)
    }

    pub(crate) fn with_connection_and_external_surface_role(
        conn: Connection,
        role: ExternalWaylandSurfaceRoleFactory,
    ) -> Self {
        let startup_activation_token = take_startup_activation_token_from_environment();
        Self::with_connection(conn, startup_activation_token, Some(role))
    }

    fn with_connection(
        conn: Connection,
        startup_activation_token: Option<String>,
        external_surface_role: Option<ExternalWaylandSurfaceRoleFactory>,
    ) -> Self {
        let (globals, mut event_queue) =
            registry_queue_init::<WaylandClientStatePtr>(&conn).unwrap();
        let qh = event_queue.handle();

        let mut seat: Option<wl_seat::WlSeat> = None;
        #[allow(clippy::mutable_key_type)]
        let mut in_progress_outputs = HashMap::default();
        #[allow(clippy::mutable_key_type)]
        let mut wl_outputs: HashMap<ObjectId, wl_output::WlOutput> = HashMap::default();
        let mut output_globals = HashMap::default();
        globals.contents().with_list(|list| {
            for global in list {
                match &global.interface[..] {
                    "wl_seat" => {
                        seat = Some(globals.registry().bind::<wl_seat::WlSeat, _, _>(
                            global.name,
                            wl_seat_version(global.version),
                            &qh,
                            (),
                        ));
                    }
                    "wl_output" => {
                        let output = globals.registry().bind::<wl_output::WlOutput, _, _>(
                            global.name,
                            wl_output_version(global.version),
                            &qh,
                            (),
                        );
                        in_progress_outputs.insert(output.id(), InProgressOutput::default());
                        output_globals.insert(global.name, output.id());
                        wl_outputs.insert(output.id(), output);
                    }
                    _ => {}
                }
            }
        });

        let event_loop = EventLoop::<WaylandClientStatePtr>::try_new().unwrap();

        let (common, main_receiver, wake_receiver, tray_receiver) =
            LinuxCommon::new(event_loop.get_signal());

        let handle = event_loop.handle();
        handle
            .insert_source(main_receiver, {
                let handle = handle.clone();
                move |event, _, _: &mut WaylandClientStatePtr| {
                    if let calloop::channel::Event::Msg(runnable) = event {
                        handle.insert_idle(|_| {
                            let location = runnable.metadata().location;
                            let spawned = runnable.metadata().spawned;
                            profiler::update_running_task(spawned, location);
                            runnable.run();
                            profiler::save_task_timing();
                        });
                    }
                }
            })
            .unwrap();

        handle
            .insert_source(
                wake_receiver,
                |event, _, client: &mut WaylandClientStatePtr| {
                    if let calloop::channel::Event::Msg(()) = event {
                        client.get_client().borrow_mut().common.handle_system_wake();
                    }
                },
            )
            .unwrap();

        handle
            .insert_source(
                tray_receiver,
                |event, _, client: &mut WaylandClientStatePtr| {
                    if let calloop::channel::Event::Msg(message) = event {
                        dispatch_tray_message(&WaylandClient(client.get_client()), message);
                    }
                },
            )
            .unwrap();

        let compositor_gpu = detect_compositor_gpu();
        let gpu_context = Rc::new(RefCell::new(None));

        let seat = seat.unwrap();
        let globals = Globals::new(
            globals,
            common.foreground_executor.clone(),
            qh.clone(),
            seat.clone(),
        );

        #[allow(clippy::mutable_key_type)]
        let mut xdg_outputs = HashMap::default();
        if let Some(manager) = &globals.xdg_output_manager {
            for (id, output) in &wl_outputs {
                let xdg_output = manager.get_xdg_output(output, &qh, id.clone());
                in_progress_outputs.get_mut(id).unwrap().xdg_version = Some(xdg_output.version());
                xdg_outputs.insert(id.clone(), xdg_output);
            }
        }

        let data_device = globals
            .data_device_manager
            .as_ref()
            .map(|data_device_manager| data_device_manager.get_data_device(&seat, &qh, ()));

        let primary_selection = globals
            .primary_selection_manager
            .as_ref()
            .map(|primary_selection_manager| primary_selection_manager.get_device(&seat, &qh, ()));

        let cursor = Cursor::new(&conn, &globals, 24);

        handle
            .insert_source(XDPEventSource::new(&common.background_executor), {
                move |event, _, client| match event {
                    XDPEvent::WindowAppearance(appearance) => {
                        if let Some(client) = client.0.upgrade() {
                            let mut client = client.borrow_mut();

                            client.common.appearance = appearance;

                            for window in client.windows.values_mut() {
                                window.set_appearance(appearance);
                            }
                        }
                    }
                    XDPEvent::ButtonLayout(layout_str) => {
                        if let Some(client) = client.0.upgrade() {
                            let layout = WindowButtonLayout::parse(&layout_str)
                                .log_err()
                                .unwrap_or_else(WindowButtonLayout::linux_default);
                            let mut client = client.borrow_mut();
                            client.common.button_layout = layout;

                            for window in client.windows.values_mut() {
                                window.set_button_layout();
                            }
                        }
                    }
                    XDPEvent::CursorTheme(theme) => {
                        if let Some(client) = client.0.upgrade() {
                            let mut client = client.borrow_mut();
                            client.cursor.set_theme(theme);
                        }
                    }
                    XDPEvent::CursorSize(size) => {
                        if let Some(client) = client.0.upgrade() {
                            let mut client = client.borrow_mut();
                            client.cursor.set_size(size);
                        }
                    }
                }
            })
            .unwrap();

        let state = Rc::new(RefCell::new(WaylandClientState {
            serial_tracker: SerialTracker::new(),
            globals,
            external_surface_role,
            gpu_context,
            compositor_gpu,
            wl_seat: seat,
            wl_pointer: None,
            wl_keyboard: None,
            pinch_gesture: None,
            pinch_scale: 1.0,
            cursor_shape_device: None,
            data_device,
            internal_drag_mime: format!(
                "application/x-gpui-internal-drag-{}",
                uuid::Uuid::new_v4().simple()
            ),
            native_drag_source: None,
            pending_drag_icons: HashMap::default(),
            primary_selection,
            text_input: None,
            pre_edit_text: None,
            ime: TextInputState::default(),
            text_input_surface: None,
            composing: false,
            outputs: HashMap::default(),
            in_progress_outputs,
            wl_outputs,
            xdg_outputs,
            output_globals,
            windows: HashMap::default(),
            common,
            keyboard_layout: LinuxKeyboardLayout::new(UNKNOWN_KEYBOARD_LAYOUT_NAME),
            keymap_state: None,
            compose_state: None,
            drag: DragState {
                data_offer: None,
                kind: None,
                window: None,
                position: Point::default(),
            },
            click: ClickState {
                last_click: Instant::now(),
                last_mouse_button: None,
                last_location: Point::default(),
                current_count: 0,
            },
            repeat: KeyRepeat {
                characters_per_second: 16,
                delay: Duration::from_millis(500),
                current_id: 0,
                current_keycode: None,
            },
            modifiers: Modifiers {
                shift: false,
                control: false,
                alt: false,
                function: false,
                platform: false,
            },
            capslock: Capslock { on: false },
            scroll_event_received: false,
            axis_source: AxisSource::Wheel,
            mouse_location: None,
            continuous_scroll_delta: None,
            discrete_scroll_delta: None,
            vertical_modifier: -1.0,
            horizontal_modifier: -1.0,
            button_pressed: None,
            mouse_focused_window: None,
            keyboard_focused_window: None,
            activation_history: Default::default(),
            loop_handle: handle.clone(),
            enter_token: None,
            cursor_style: None,
            cursor_hidden_window: None,
            clipboard: Clipboard::new(conn.clone(), handle.clone()),
            data_offers: Vec::new(),
            primary_data_offer: None,
            cursor,
            startup_activation_token,
            event_loop: Some(event_loop),
            ime_enabled: None,
        }));

        // The registry roundtrip only discovers globals. Dispatch the initial
        // properties of the outputs bound above before applications query displays.
        event_queue
            .roundtrip(&mut WaylandClientStatePtr(Rc::downgrade(&state)))
            .expect("failed to initialize Wayland output information");

        insert_wayland_source(conn, event_queue, handle).unwrap();

        Self(state)
    }
}

impl LinuxClient for WaylandClient {
    fn keyboard_layout(&self) -> Box<dyn PlatformKeyboardLayout> {
        Box::new(self.0.borrow().keyboard_layout.clone())
    }

    fn displays(&self) -> Vec<Rc<dyn PlatformDisplay>> {
        self.0
            .borrow()
            .outputs
            .iter()
            .map(|(id, output)| {
                Rc::new(WaylandDisplay {
                    id: id.clone(),
                    name: output.name.clone(),
                    scale_factor: output.display_scale_factor(),
                    bounds: output.logical_bounds(),
                }) as Rc<dyn PlatformDisplay>
            })
            .collect()
    }

    fn display(&self, id: DisplayId) -> Option<Rc<dyn PlatformDisplay>> {
        self.0
            .borrow()
            .outputs
            .iter()
            .find_map(|(object_id, output)| {
                (object_id.protocol_id() as u64 == u64::from(id)).then(|| {
                    Rc::new(WaylandDisplay {
                        id: object_id.clone(),
                        name: output.name.clone(),
                        scale_factor: output.display_scale_factor(),
                        bounds: output.logical_bounds(),
                    }) as Rc<dyn PlatformDisplay>
                })
            })
    }

    fn primary_display(&self) -> Option<Rc<dyn PlatformDisplay>> {
        None
    }

    #[cfg(feature = "screen-capture")]
    fn screen_capture_sources(
        &self,
    ) -> futures::channel::oneshot::Receiver<anyhow::Result<Vec<Rc<dyn gpui::ScreenCaptureSource>>>>
    {
        gpui::scap_screen_capture::start_scap_default_target_source(
            &self.0.borrow().common.foreground_executor,
        )
    }

    fn open_window(
        &self,
        handle: AnyWindowHandle,
        params: WindowParams,
    ) -> anyhow::Result<Box<dyn PlatformWindow>> {
        let mut state = self.0.borrow_mut();

        // Popups name their parent explicitly. Other kinds are parented to the focused window.
        let (parent, popup_grab) = match &params.kind {
            WindowKind::AnchoredPopup(options) => {
                let parent = state
                    .windows
                    .values()
                    .find(|window| window.handle() == options.parent)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("popup parent window not found"))?;
                // A popup grab must reference a press event or the compositor declines it and
                // immediately dismisses the popup, so use the most recent press serial, or no
                // grab before any press.
                let popup_grab = options.grab.then(|| {
                    let serial = state
                        .serial_tracker
                        .get(SerialKind::MousePress)
                        .max(state.serial_tracker.get(SerialKind::KeyPress));
                    (serial != 0).then(|| (serial, state.wl_seat.clone()))
                });
                (Some(parent), popup_grab.flatten())
            }
            _ => (state.keyboard_focused_window.clone(), None),
        };

        let target_output = params
            .display_id
            .and_then(|display_id| state.output_for_display(display_id));

        let appearance = state.common.appearance;
        let compositor_gpu = state.compositor_gpu.take();

        let activation_target = matches!(
            params.kind,
            WindowKind::Normal | WindowKind::Floating | WindowKind::Dialog
        );
        let (window, surface_id) = WaylandWindow::new(
            handle,
            state.globals.clone(),
            state.gpu_context.clone(),
            compositor_gpu,
            WaylandClientStatePtr(Rc::downgrade(&self.0)),
            params,
            appearance,
            parent,
            popup_grab,
            target_output,
            state.external_surface_role.as_ref(),
        )?;

        if window.0.toplevel().is_some() {
            if activation_target {
                state.activation_history.opened(surface_id.clone());
            }
            state.consume_startup_activation_token(&window.0.surface());
        }
        state.windows.insert(surface_id, window.0.clone());

        Ok(Box::new(window))
    }

    fn set_cursor_style(&self, style: CursorStyle) {
        let mut state = self.0.borrow_mut();

        let need_update = state.cursor_style != Some(style)
            && (state.mouse_focused_window.is_none()
                || state
                    .mouse_focused_window
                    .as_ref()
                    .is_some_and(|w| !w.is_blocked()));

        if !need_update {
            return;
        }

        state.cursor_style = Some(style);

        // Don't clobber the invisible cursor; restore reads back from `cursor_style`.
        if state.cursor_hidden_window.is_some() {
            return;
        }

        let serial = state.serial_tracker.get(SerialKind::MouseEnter);
        if let Some(cursor_shape_device) = &state.cursor_shape_device {
            cursor_shape_device.set_shape(serial, to_shape(style));
        } else if let Some(focused_window) = &state.mouse_focused_window {
            // cursor-shape-v1 isn't supported, set the cursor using a surface.
            let wl_pointer = state
                .wl_pointer
                .clone()
                .expect("window is focused by pointer");
            let scale = focused_window.primary_output_scale();
            state.cursor.set_icon(
                &wl_pointer,
                serial,
                cursor_style_to_icon_names(style),
                scale,
            );
        }
    }

    fn hide_cursor_until_mouse_moves(&self) {
        self.0.borrow_mut().hide_cursor_until_mouse_moves();
    }

    fn is_cursor_visible(&self) -> bool {
        self.0.borrow().cursor_hidden_window.is_none()
    }

    fn open_uri(&self, uri: &str) {
        let state = self.0.borrow();
        if let (Some(activation), Some(window)) = (
            state.globals.activation.clone(),
            state.mouse_focused_window.clone(),
        ) {
            let token = activation
                .get_activation_token(&state.globals.qh, PendingActivation::Uri(uri.to_string()));
            let serial = state.serial_tracker.get(SerialKind::MousePress);
            token.set_serial(serial, &state.wl_seat);
            token.set_surface(&window.surface());
            token.commit();
        } else {
            let executor = state.common.background_executor.clone();
            open_uri_internal(executor, uri, None);
        }
    }

    fn reveal_path(&self, path: PathBuf) {
        let state = self.0.borrow();
        if let (Some(activation), Some(window)) = (
            state.globals.activation.clone(),
            state.mouse_focused_window.clone(),
        ) {
            let token =
                activation.get_activation_token(&state.globals.qh, PendingActivation::Path(path));
            let serial = state.serial_tracker.get(SerialKind::MousePress);
            token.set_serial(serial, &state.wl_seat);
            token.set_surface(&window.surface());
            token.commit();
        } else {
            let executor = state.common.background_executor.clone();
            reveal_path_internal(executor, path, None);
        }
    }

    fn with_common<R>(&self, f: impl FnOnce(&mut LinuxCommon) -> R) -> R {
        f(&mut self.0.borrow_mut().common)
    }

    fn run(&self) {
        let mut event_loop = self
            .0
            .borrow_mut()
            .event_loop
            .take()
            .expect("App is already running");

        event_loop
            .run(
                None,
                &mut WaylandClientStatePtr(Rc::downgrade(&self.0)),
                |_| {},
            )
            .log_err();
    }

    fn write_to_primary(&self, item: gpui::ClipboardItem) {
        let mut state = self.0.borrow_mut();
        let (Some(primary_selection_manager), Some(primary_selection)) = (
            state.globals.primary_selection_manager.clone(),
            state.primary_selection.clone(),
        ) else {
            return;
        };
        if state.mouse_focused_window.is_some() || state.keyboard_focused_window.is_some() {
            let source = match crate::linux::clipboard::ClipboardSource::new(&item) {
                Ok(source) => source,
                Err(error) => {
                    log::warn!("cannot write file clipboard: {error:#}");
                    return;
                }
            };
            state.clipboard.set_primary(item);
            let serial = state.serial_tracker.get_latest();
            let data_source =
                primary_selection_manager.create_source(&state.globals.qh, source.clone());
            for (mime_type, _) in &source.0 {
                data_source.offer(mime_type.to_string());
            }
            data_source.offer(state.clipboard.self_mime());
            primary_selection.set_selection(Some(&data_source), serial);
        }
    }

    fn write_to_clipboard(&self, item: gpui::ClipboardItem) {
        let mut state = self.0.borrow_mut();
        let (Some(data_device_manager), Some(data_device)) = (
            state.globals.data_device_manager.clone(),
            state.data_device.clone(),
        ) else {
            return;
        };
        if state.mouse_focused_window.is_some() || state.keyboard_focused_window.is_some() {
            let source = match crate::linux::clipboard::ClipboardSource::new(&item) {
                Ok(source) => source,
                Err(error) => {
                    log::warn!("cannot write file clipboard: {error:#}");
                    return;
                }
            };
            state.clipboard.set(item);
            let serial = state.serial_tracker.get_latest();
            let data_source = data_device_manager.create_data_source(
                &state.globals.qh,
                WaylandDataSourceKind::Clipboard(source.clone()),
            );
            for (mime_type, _) in &source.0 {
                data_source.offer(mime_type.to_string());
            }
            data_source.offer(state.clipboard.self_mime());
            data_device.set_selection(Some(&data_source), serial);
        }
    }

    fn read_from_primary(&self) -> Option<gpui::ClipboardItem> {
        self.0.borrow_mut().clipboard.read_primary()
    }

    fn read_from_clipboard(&self) -> Option<gpui::ClipboardItem> {
        self.0.borrow_mut().clipboard.read()
    }

    fn active_window(&self) -> Option<AnyWindowHandle> {
        self.0
            .borrow_mut()
            .keyboard_focused_window
            .as_ref()
            .map(|window| window.handle())
    }

    fn activate(&self) {
        let window = {
            let state = self.0.borrow();
            state
                .activation_history
                .target()
                .and_then(|id| state.windows.get(id))
                .cloned()
        };
        if let Some(window) = window {
            window.activate();
        }
    }

    fn window_stack(&self) -> Option<Vec<AnyWindowHandle>> {
        None
    }

    fn compositor_name(&self) -> &'static str {
        "Wayland"
    }

    fn window_identifier(&self) -> impl Future<Output = Option<WindowIdentifier>> + Send + 'static {
        async fn inner(surface: Option<wl_surface::WlSurface>) -> Option<WindowIdentifier> {
            if let Some(surface) = surface {
                ashpd::WindowIdentifier::from_wayland(&surface).await
            } else {
                None
            }
        }

        let client_state = self.0.borrow();
        let active_window = client_state.keyboard_focused_window.as_ref();
        inner(active_window.map(|aw| aw.surface()))
    }
}

struct DmabufProbeState {
    device: Option<u64>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for DmabufProbeState {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1, ()> for DmabufProbeState {
    fn event(
        _: &mut Self,
        _: &zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1,
        _: zwp_linux_dmabuf_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1, ()> for DmabufProbeState {
    fn event(
        state: &mut Self,
        _: &zwp_linux_dmabuf_feedback_v1::ZwpLinuxDmabufFeedbackV1,
        event: zwp_linux_dmabuf_feedback_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwp_linux_dmabuf_feedback_v1::Event::MainDevice { device } = event {
            if let Ok(bytes) = <[u8; 8]>::try_from(device.as_slice()) {
                state.device = Some(u64::from_ne_bytes(bytes));
            }
        }
    }
}

fn detect_compositor_gpu() -> Option<CompositorGpuHint> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut event_queue) = registry_queue_init::<DmabufProbeState>(&connection).ok()?;
    let queue_handle = event_queue.handle();

    let dmabuf: zwp_linux_dmabuf_v1::ZwpLinuxDmabufV1 =
        globals.bind(&queue_handle, 4..=4, ()).ok()?;
    let feedback = dmabuf.get_default_feedback(&queue_handle, ());

    let mut state = DmabufProbeState { device: None };

    event_queue.roundtrip(&mut state).ok()?;

    feedback.destroy();
    dmabuf.destroy();

    crate::linux::compositor_gpu_hint_from_dev_t(state.device?)
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match &interface[..] {
                "wl_seat" => {
                    if let Some(wl_pointer) = state.wl_pointer.take() {
                        wl_pointer.release();
                    }
                    if let Some(wl_keyboard) = state.wl_keyboard.take() {
                        wl_keyboard.release();
                    }
                    state.wl_seat.release();
                    state.wl_seat = registry.bind::<wl_seat::WlSeat, _, _>(
                        name,
                        wl_seat_version(version),
                        qh,
                        (),
                    );
                }
                "wl_output" => {
                    let output = registry.bind::<wl_output::WlOutput, _, _>(
                        name,
                        wl_output_version(version),
                        qh,
                        (),
                    );

                    let mut pending = InProgressOutput::default();
                    if let Some(manager) = &state.globals.xdg_output_manager {
                        let xdg_output = manager.get_xdg_output(&output, qh, output.id());
                        pending.xdg_version = Some(xdg_output.version());
                        state.xdg_outputs.insert(output.id(), xdg_output);
                    }
                    state.in_progress_outputs.insert(output.id(), pending);
                    state.output_globals.insert(name, output.id());
                    state.wl_outputs.insert(output.id(), output);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                let Some(id) = state.output_globals.remove(&name) else {
                    return;
                };
                state.in_progress_outputs.remove(&id);
                state.outputs.remove(&id);
                if let Some(output) = state.xdg_outputs.remove(&id) {
                    output.destroy();
                }
                if let Some(output) = state.wl_outputs.remove(&id)
                    && output.version() >= wl_output::REQ_RELEASE_SINCE
                {
                    output.release();
                }
                let windows = state.windows.values().cloned().collect::<Vec<_>>();
                // Window callbacks can re-enter the client to query displays.
                drop(state);
                for window in windows {
                    window.remove_output(&id);
                }
            }
            _ => {}
        }
    }
}

delegate_noop!(WaylandClientStatePtr: ignore xdg_activation_v1::XdgActivationV1);
delegate_noop!(WaylandClientStatePtr: ignore xdg_system_bell_v1::XdgSystemBellV1);
delegate_noop!(WaylandClientStatePtr: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore wl_compositor::WlCompositor);
delegate_noop!(WaylandClientStatePtr: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);
delegate_noop!(WaylandClientStatePtr: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore wl_data_device_manager::WlDataDeviceManager);
delegate_noop!(WaylandClientStatePtr: ignore zwp_primary_selection_device_manager_v1::ZwpPrimarySelectionDeviceManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore wl_shm::WlShm);
delegate_noop!(WaylandClientStatePtr: ignore wl_shm_pool::WlShmPool);
delegate_noop!(WaylandClientStatePtr: ignore wl_buffer::WlBuffer);
delegate_noop!(WaylandClientStatePtr: ignore wl_region::WlRegion);
delegate_noop!(WaylandClientStatePtr: ignore wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore zxdg_decoration_manager_v1::ZxdgDecorationManagerV1);
delegate_noop!(WaylandClientStatePtr: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(WaylandClientStatePtr: ignore xdg_positioner::XdgPositioner);
delegate_noop!(WaylandClientStatePtr: ignore org_kde_kwin_blur_manager::OrgKdeKwinBlurManager);
delegate_noop!(WaylandClientStatePtr: ignore zwp_text_input_manager_v3::ZwpTextInputManagerV3);
delegate_noop!(WaylandClientStatePtr: ignore org_kde_kwin_blur::OrgKdeKwinBlur);
delegate_noop!(WaylandClientStatePtr: ignore wp_viewporter::WpViewporter);
delegate_noop!(WaylandClientStatePtr: ignore wp_viewport::WpViewport);

impl Dispatch<WlCallback, ObjectId> for WaylandClientStatePtr {
    fn event(
        state: &mut WaylandClientStatePtr,
        callback: &wl_callback::WlCallback,
        event: wl_callback::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = state.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };
        drop(state);

        if let wl_callback::Event::Done { .. } = event {
            window.frame_done(&callback.id());
        }
    }
}

pub(crate) fn get_window(
    state: &mut RefMut<WaylandClientState>,
    surface_id: &ObjectId,
) -> Option<WaylandWindowStatePtr> {
    state.windows.get(surface_id).cloned()
}

impl Dispatch<wl_surface::WlSurface, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        surface: &wl_surface::WlSurface,
        event: <wl_surface::WlSurface as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(window) = get_window(&mut state, &surface.id()) else {
            return;
        };
        #[allow(clippy::mutable_key_type)]
        let outputs = state.outputs.clone();
        drop(state);

        window.handle_surface_event(event, outputs);
    }
}

impl Dispatch<wl_output::WlOutput, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        output: &wl_output::WlOutput,
        event: <wl_output::WlOutput as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(in_progress_output) = state.in_progress_outputs.get_mut(&output.id()) else {
            return;
        };
        let Some(complete) = in_progress_output.apply(event) else {
            return;
        };
        drop(state);
        this.publish_output(&output.id(), complete);
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        output_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(pending) = state.in_progress_outputs.get_mut(output_id) else {
            return;
        };
        let Some(complete) = pending.apply_xdg(event) else {
            return;
        };
        drop(state);
        this.publish_output(output_id, complete);
    }
}

impl Dispatch<xdg_surface::XdgSurface, ObjectId> for WaylandClientStatePtr {
    fn event(
        state: &mut Self,
        _: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = state.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };
        drop(state);
        window.handle_xdg_surface_event(event);
    }
}

impl Dispatch<xdg_toplevel::XdgToplevel, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: <xdg_toplevel::XdgToplevel as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        let should_close = window.handle_toplevel_event(event);

        if should_close {
            // The close logic will be handled in drop_window()
            window.close();
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: <zwlr_layer_surface_v1::ZwlrLayerSurfaceV1 as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        let should_close = window.handle_layersurface_event(event);

        if should_close {
            // Close logic will be handled in drop_window()
            window.close();
        }
    }
}

impl Dispatch<xdg_popup::XdgPopup, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &xdg_popup::XdgPopup,
        event: <xdg_popup::XdgPopup as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        let should_close = window.handle_popup_event(event);

        if should_close {
            // The close logic will be handled in drop_window()
            window.close();
        }
    }
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        wm_base: &xdg_wm_base::XdgWmBase,
        event: <xdg_wm_base::XdgWmBase as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm_base.pong(serial);
        }
    }
}

impl Dispatch<xdg_activation_token_v1::XdgActivationTokenV1, PendingActivation>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        token: &xdg_activation_token_v1::XdgActivationTokenV1,
        event: <xdg_activation_token_v1::XdgActivationTokenV1 as Proxy>::Event,
        request: &PendingActivation,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        if let xdg_activation_token_v1::Event::Done { token } = event {
            let executor = state.common.background_executor.clone();
            match request {
                PendingActivation::Uri(uri) => open_uri_internal(executor, uri, Some(token)),
                PendingActivation::Path(path) => {
                    reveal_path_internal(executor, path.clone(), Some(token))
                }
                PendingActivation::Window(window) => {
                    if let Some(window) = get_window(&mut state, window)
                        && let Some(activation) = &state.globals.activation
                    {
                        activation.activate(token, &window.surface());
                    }
                }
            }
        }

        token.destroy();
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for WaylandClientStatePtr {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        {
            let client = state.get_client();
            let mut state = client.borrow_mut();
            if capabilities.contains(wl_seat::Capability::Keyboard) {
                let keyboard = seat.get_keyboard(qh, ());

                if let Some(text_input) = state.text_input.take() {
                    text_input.destroy();
                    state.ime = TextInputState::default();
                    state.text_input_surface = None;
                    state.composing = false;
                }

                state.text_input = state
                    .globals
                    .text_input_manager
                    .as_ref()
                    .map(|text_input_manager| text_input_manager.get_text_input(seat, qh, ()));

                if let Some(wl_keyboard) = &state.wl_keyboard {
                    wl_keyboard.release();
                }

                state.wl_keyboard = Some(keyboard);
            }
            if capabilities.contains(wl_seat::Capability::Pointer) {
                let pointer = seat.get_pointer(qh, ());

                if let Some(cursor_shape_device) = state.cursor_shape_device.take() {
                    cursor_shape_device.destroy();
                }

                state.cursor_shape_device = state
                    .globals
                    .cursor_shape_manager
                    .as_ref()
                    .map(|cursor_shape_manager| cursor_shape_manager.get_pointer(&pointer, qh, ()));

                state.pinch_gesture = state.globals.gesture_manager.as_ref().map(
                    |gesture_manager: &zwp_pointer_gestures_v1::ZwpPointerGesturesV1| {
                        gesture_manager.get_pinch_gesture(&pointer, qh, ())
                    },
                );

                if let Some(wl_pointer) = &state.wl_pointer {
                    wl_pointer.release();
                }

                state.wl_pointer = Some(pointer);
            }
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        match event {
            wl_keyboard::Event::RepeatInfo { rate, delay } => {
                state.repeat.characters_per_second = rate as u32;
                state.repeat.delay = Duration::from_millis(delay as u64);
            }
            wl_keyboard::Event::Keymap {
                format, fd, size, ..
            } => {
                let xkb_context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
                state.keymap_state = load_keymap(&xkb_context, format, fd, size).log_err();
                state.compose_state = state
                    .keymap_state
                    .as_ref()
                    .and_then(|_| get_xkb_compose_state(&xkb_context));
                state.repeat.current_id += 1;
                state.repeat.current_keycode = None;
                drop(state);

                this.handle_keyboard_layout_change();
            }
            wl_keyboard::Event::Enter { surface, .. } => {
                state.activation_history.focused(&surface.id());
                state.keyboard_focused_window = get_window(&mut state, &surface.id());
                state.enter_token = Some(());

                if let Some(window) = state.keyboard_focused_window.clone() {
                    drop(state);
                    window.set_focused(true);
                }
            }
            wl_keyboard::Event::Leave { surface, .. } => {
                let keyboard_focused_window = get_window(&mut state, &surface.id());
                state.keyboard_focused_window = None;
                state.enter_token.take();
                // Prevent keyboard events from repeating after opening e.g. a file chooser and closing it quickly
                state.repeat.current_id += 1;
                state.restore_cursor_after_hide();

                if let Some(window) = keyboard_focused_window {
                    if let Some(ref mut compose) = state.compose_state {
                        compose.reset();
                    }
                    state.pre_edit_text.take();
                    drop(state);
                    window.handle_ime(ImeInput::DeleteText);
                    window.set_focused(false);
                }
            }
            wl_keyboard::Event::Modifiers {
                mods_depressed,
                mods_latched,
                mods_locked,
                group,
                ..
            } => {
                let focused_window = state.keyboard_focused_window.clone();

                let Some((old_layout, modifiers, capslock)) = update_modifiers(
                    state.keymap_state.as_mut(),
                    mods_depressed,
                    mods_latched,
                    mods_locked,
                    group,
                ) else {
                    return;
                };
                state.modifiers = modifiers;
                state.capslock = capslock;

                let input = PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers: state.modifiers,
                    capslock: state.capslock,
                });
                drop(state);

                if let Some(focused_window) = focused_window {
                    focused_window.handle_input(input);
                }

                if group != old_layout {
                    this.handle_keyboard_layout_change();
                }
            }
            wl_keyboard::Event::Key {
                serial,
                key,
                state: WEnum::Value(key_state),
                ..
            } => {
                state.serial_tracker.update(SerialKind::KeyPress, serial);

                let focused_window = state.keyboard_focused_window.clone();
                let Some(focused_window) = focused_window else {
                    return;
                };

                let Some((keycode, keysym, mut keystroke)) =
                    translate_key(state.keymap_state.as_ref(), state.modifiers, key)
                else {
                    return;
                };

                match key_state {
                    wl_keyboard::KeyState::Pressed if !keysym.is_modifier_key() => {
                        if let Some(mut compose) = state.compose_state.take() {
                            compose.feed(keysym);
                            match compose.status() {
                                xkb::Status::Composing => {
                                    keystroke.key_char = None;
                                    state.pre_edit_text =
                                        compose.utf8().or(keystroke_underlying_dead_key(keysym));
                                    let pre_edit =
                                        state.pre_edit_text.clone().unwrap_or(String::default());
                                    drop(state);
                                    focused_window.handle_ime(ImeInput::SetMarkedText(pre_edit));
                                    state = client.borrow_mut();
                                }

                                xkb::Status::Composed => {
                                    state.pre_edit_text.take();
                                    keystroke.key_char = compose.utf8();
                                    if let Some(keysym) = compose.keysym() {
                                        keystroke.key = xkb::keysym_get_name(keysym);
                                    }
                                }
                                xkb::Status::Cancelled => {
                                    let pre_edit = state.pre_edit_text.take();
                                    let new_pre_edit = keystroke_underlying_dead_key(keysym);
                                    state.pre_edit_text = new_pre_edit.clone();
                                    drop(state);
                                    if let Some(pre_edit) = pre_edit {
                                        focused_window.handle_ime(ImeInput::InsertText(pre_edit));
                                    }
                                    if let Some(current_key) = new_pre_edit {
                                        focused_window
                                            .handle_ime(ImeInput::SetMarkedText(current_key));
                                    }
                                    compose.feed(keysym);
                                    state = client.borrow_mut();
                                }
                                _ => {}
                            }
                            state.compose_state = Some(compose);
                        }
                        let input = PlatformInput::KeyDown(KeyDownEvent {
                            keystroke: keystroke.clone(),
                            is_held: false,
                            prefer_character_input: false,
                        });

                        state.repeat.current_id += 1;
                        state.repeat.current_keycode = Some(keycode);

                        let rate = state.repeat.characters_per_second;
                        let repeat_interval = Duration::from_secs(1) / rate.max(1);
                        let id = state.repeat.current_id;
                        state
                            .loop_handle
                            .insert_source(Timer::from_duration(state.repeat.delay), {
                                let input = PlatformInput::KeyDown(KeyDownEvent {
                                    keystroke,
                                    is_held: true,
                                    prefer_character_input: false,
                                });
                                move |event_timestamp, _metadata, this| {
                                    let client = this.get_client();
                                    let state = client.borrow();
                                    let is_repeating = id == state.repeat.current_id
                                        && state.repeat.current_keycode.is_some()
                                        && state.keyboard_focused_window.is_some();

                                    if !is_repeating || rate == 0 {
                                        return TimeoutAction::Drop;
                                    }

                                    let focused_window =
                                        state.keyboard_focused_window.as_ref().unwrap().clone();

                                    drop(state);
                                    focused_window.handle_input(input.clone());

                                    // If the new scheduled time is in the past the event will repeat as soon as possible
                                    TimeoutAction::ToInstant(event_timestamp + repeat_interval)
                                }
                            })
                            .unwrap();

                        drop(state);
                        focused_window.handle_input(input);
                    }
                    wl_keyboard::KeyState::Released if !keysym.is_modifier_key() => {
                        let input = PlatformInput::KeyUp(KeyUpEvent { keystroke });

                        if state.repeat.current_keycode == Some(keycode) {
                            state.repeat.current_keycode = None;
                        }

                        drop(state);
                        focused_window.handle_input(input);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_text_input_v3::ZwpTextInputV3, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        text_input: &zwp_text_input_v3::ZwpTextInputV3,
        event: <zwp_text_input_v3::ZwpTextInputV3 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        if state.text_input.as_ref() != Some(text_input) {
            return;
        }
        match event {
            zwp_text_input_v3::Event::Enter { surface } => {
                state.text_input_surface = Some(surface.id());
                drop(state);
                this.enable_ime();
            }
            zwp_text_input_v3::Event::Leave { surface } => {
                state.text_input_surface = None;
                let window = state.windows.get(&surface.id()).cloned();
                drop(state);
                this.disable_ime();
                if let Some(window) = window {
                    window.handle_ime(ImeInput::DeleteText);
                }
            }
            zwp_text_input_v3::Event::CommitString { text } => {
                state.ime.pending.commit = text;
            }
            zwp_text_input_v3::Event::DeleteSurroundingText {
                before_length,
                after_length,
            } => {
                state.ime.pending.delete = Some((before_length, after_length));
            }
            zwp_text_input_v3::Event::PreeditString {
                text,
                cursor_begin,
                cursor_end,
            } => {
                state.ime.pending.preedit = Some(Preedit::new(text, cursor_begin, cursor_end));
            }
            zwp_text_input_v3::Event::Done { serial } => {
                let batch = state.ime.take_pending();
                let Some(window) = state.keyboard_focused_window.clone() else {
                    return;
                };
                if state.ime_enabled != Some(true)
                    || state.text_input_surface.as_ref() != Some(&window.surface().id())
                {
                    return;
                }
                let expected = state.ime.deletion_context(serial).cloned();
                state.ime.defer_publish = !state.ime.can_publish(serial);
                drop(state);
                let Some((context, composing)) = window.handle_ime_batch(batch, expected.as_ref())
                else {
                    return;
                };
                let area = window.get_ime_area();
                {
                    let mut state = client.borrow_mut();
                    state.composing = composing;
                    if state.ime.can_publish(serial)
                        && state.ime_enabled == Some(true)
                        && state.text_input_surface.as_ref() == Some(&window.surface().id())
                    {
                        let mut changed = state.publish_surrounding(
                            text_input,
                            context,
                            ChangeCause::InputMethod,
                        );
                        if let Some([x, y, width, height]) =
                            area.and_then(|area| state.ime.update_cursor_rectangle(area))
                        {
                            text_input.set_cursor_rectangle(x, y, width, height);
                            changed = true;
                        }
                        if changed {
                            text_input.commit();
                            state.ime.committed();
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn linux_button_to_gpui(button: u32) -> Option<MouseButton> {
    // These values are coming from <linux/input-event-codes.h>.
    const BTN_LEFT: u32 = 0x110;
    const BTN_RIGHT: u32 = 0x111;
    const BTN_MIDDLE: u32 = 0x112;
    const BTN_SIDE: u32 = 0x113;
    const BTN_EXTRA: u32 = 0x114;
    const BTN_FORWARD: u32 = 0x115;
    const BTN_BACK: u32 = 0x116;

    Some(match button {
        BTN_LEFT => MouseButton::Left,
        BTN_RIGHT => MouseButton::Right,
        BTN_MIDDLE => MouseButton::Middle,
        BTN_BACK | BTN_SIDE => MouseButton::Navigate(NavigationDirection::Back),
        BTN_FORWARD | BTN_EXTRA => MouseButton::Navigate(NavigationDirection::Forward),
        _ => return None,
    })
}

impl Dispatch<wl_pointer::WlPointer, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        wl_pointer: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                let position = point(px(surface_x as f32), px(surface_y as f32));
                state.serial_tracker.update(SerialKind::MouseEnter, serial);
                state.mouse_location = Some(position);
                state.button_pressed = None;

                if let Some(window) = get_window(&mut state, &surface.id()) {
                    state.mouse_focused_window = Some(window.clone());

                    if state.enter_token.is_some() {
                        state.enter_token = None;
                    }
                    state.restore_cursor_after_hide();
                    if let Some(style) = state.cursor_style {
                        if let Some(cursor_shape_device) = &state.cursor_shape_device {
                            cursor_shape_device.set_shape(serial, to_shape(style));
                        } else {
                            let scale = window.primary_output_scale();
                            state.cursor.set_icon(
                                wl_pointer,
                                serial,
                                cursor_style_to_icon_names(style),
                                scale,
                            );
                        }
                    }
                    let modifiers = state.modifiers;
                    drop(state);
                    window.set_hovered(true);
                    // No Motion follows Enter unless the pointer keeps moving, so synthesize
                    // a MouseMove to establish hover at the entry position.
                    window.handle_input(PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        pressed_button: None,
                        modifiers,
                    }));
                }
            }
            wl_pointer::Event::Leave { .. } => {
                if let Some(focused_window) = state.mouse_focused_window.clone() {
                    let input = PlatformInput::MouseExited(MouseExitEvent {
                        position: state.mouse_location.unwrap(),
                        pressed_button: state.button_pressed,
                        modifiers: state.modifiers,
                    });
                    state.mouse_focused_window = None;
                    state.mouse_location = None;
                    state.button_pressed = None;
                    state.cursor_hidden_window = None;

                    drop(state);
                    focused_window.handle_input(input);
                    focused_window.set_hovered(false);
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if state.mouse_focused_window.is_none() {
                    return;
                }
                state.mouse_location = Some(point(px(surface_x as f32), px(surface_y as f32)));
                state.restore_cursor_after_hide();

                if let Some(window) = state.mouse_focused_window.clone() {
                    if window.is_blocked() {
                        let default_style = CursorStyle::Arrow;
                        if state.cursor_style != Some(default_style) {
                            let serial = state.serial_tracker.get(SerialKind::MouseEnter);
                            state.cursor_style = Some(default_style);

                            if let Some(cursor_shape_device) = &state.cursor_shape_device {
                                cursor_shape_device.set_shape(serial, to_shape(default_style));
                            } else {
                                // cursor-shape-v1 isn't supported, set the cursor using a surface.
                                let wl_pointer = state
                                    .wl_pointer
                                    .clone()
                                    .expect("window is focused by pointer");
                                let scale = window.primary_output_scale();
                                state.cursor.set_icon(
                                    &wl_pointer,
                                    serial,
                                    cursor_style_to_icon_names(default_style),
                                    scale,
                                );
                            }
                        }
                    }
                    if state
                        .keyboard_focused_window
                        .as_ref()
                        .is_some_and(|keyboard_window| window.ptr_eq(keyboard_window))
                    {
                        state.enter_token = None;
                    }
                    let input = PlatformInput::MouseMove(MouseMoveEvent {
                        position: state.mouse_location.unwrap(),
                        pressed_button: state.button_pressed,
                        modifiers: state.modifiers,
                    });
                    drop(state);
                    window.handle_input(input);
                }
            }
            wl_pointer::Event::Button {
                serial,
                button,
                state: WEnum::Value(button_state),
                ..
            } => {
                // Record presses only. Requests referencing this serial (popup grabs,
                // interactive moves) are declined when given a release serial.
                if button_state == wl_pointer::ButtonState::Pressed {
                    state.serial_tracker.update(SerialKind::MousePress, serial);
                }
                let button = linux_button_to_gpui(button);
                let Some(button) = button else { return };
                if state.mouse_focused_window.is_none() {
                    return;
                }
                match button_state {
                    wl_pointer::ButtonState::Pressed => {
                        if let Some(window) = state.keyboard_focused_window.clone() {
                            if state.composing && state.text_input.is_some() {
                                drop(state);
                                // text_input_v3 don't have something like a reset function
                                this.disable_ime();
                                this.enable_ime();
                                window.handle_ime(ImeInput::UnmarkText);
                                state = client.borrow_mut();
                            } else if let (Some(text), Some(compose)) =
                                (state.pre_edit_text.take(), state.compose_state.as_mut())
                            {
                                compose.reset();
                                drop(state);
                                window.handle_ime(ImeInput::InsertText(text));
                                state = client.borrow_mut();
                            }
                        }
                        let click_elapsed = state.click.last_click.elapsed();

                        if click_elapsed < DOUBLE_CLICK_INTERVAL
                            && state
                                .click
                                .last_mouse_button
                                .is_some_and(|prev_button| prev_button == button)
                            && is_within_click_distance(
                                state.click.last_location,
                                state.mouse_location.unwrap(),
                            )
                        {
                            state.click.current_count += 1;
                        } else {
                            state.click.current_count = 1;
                        }

                        state.click.last_click = Instant::now();
                        state.click.last_mouse_button = Some(button);
                        state.click.last_location = state.mouse_location.unwrap();

                        state.button_pressed = Some(button);

                        if let Some(window) = state.mouse_focused_window.clone() {
                            let input = PlatformInput::MouseDown(MouseDownEvent {
                                button,
                                position: state.mouse_location.unwrap(),
                                modifiers: state.modifiers,
                                click_count: state.click.current_count,
                                first_mouse: state.enter_token.take().is_some(),
                            });
                            drop(state);
                            window.handle_input(input);
                        }
                    }
                    wl_pointer::ButtonState::Released => {
                        state.button_pressed = None;

                        if let Some(window) = state.mouse_focused_window.clone() {
                            let input = PlatformInput::MouseUp(MouseUpEvent {
                                button,
                                position: state.mouse_location.unwrap(),
                                modifiers: state.modifiers,
                                click_count: state.click.current_count,
                            });
                            drop(state);
                            window.handle_input(input);
                        }
                    }
                    _ => {}
                }
            }

            // Axis Events
            wl_pointer::Event::AxisSource {
                axis_source: WEnum::Value(axis_source),
            } => {
                state.axis_source = axis_source;
            }
            wl_pointer::Event::Axis {
                axis: WEnum::Value(axis),
                value,
                ..
            } => {
                if state.axis_source == AxisSource::Wheel {
                    return;
                }
                let axis = if state.modifiers.shift {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    axis
                };
                let axis_modifier = match axis {
                    wl_pointer::Axis::VerticalScroll => state.vertical_modifier,
                    wl_pointer::Axis::HorizontalScroll => state.horizontal_modifier,
                    _ => 1.0,
                };
                state.scroll_event_received = true;
                let scroll_delta = state
                    .continuous_scroll_delta
                    .get_or_insert(point(px(0.0), px(0.0)));
                let modifier = 3.0;
                match axis {
                    wl_pointer::Axis::VerticalScroll => {
                        scroll_delta.y += px(value as f32 * modifier * axis_modifier);
                    }
                    wl_pointer::Axis::HorizontalScroll => {
                        scroll_delta.x += px(value as f32 * modifier * axis_modifier);
                    }
                    _ => unreachable!(),
                }
            }
            wl_pointer::Event::AxisDiscrete {
                axis: WEnum::Value(axis),
                discrete,
            } => {
                state.scroll_event_received = true;
                let axis = if state.modifiers.shift {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    axis
                };
                let axis_modifier = match axis {
                    wl_pointer::Axis::VerticalScroll => state.vertical_modifier,
                    wl_pointer::Axis::HorizontalScroll => state.horizontal_modifier,
                    _ => 1.0,
                };

                let scroll_delta = state.discrete_scroll_delta.get_or_insert(point(0.0, 0.0));
                match axis {
                    wl_pointer::Axis::VerticalScroll => {
                        scroll_delta.y += discrete as f32 * axis_modifier * SCROLL_LINES;
                    }
                    wl_pointer::Axis::HorizontalScroll => {
                        scroll_delta.x += discrete as f32 * axis_modifier * SCROLL_LINES;
                    }
                    _ => unreachable!(),
                }
            }
            wl_pointer::Event::AxisValue120 {
                axis: WEnum::Value(axis),
                value120,
            } => {
                state.scroll_event_received = true;
                let axis = if state.modifiers.shift {
                    wl_pointer::Axis::HorizontalScroll
                } else {
                    axis
                };
                let axis_modifier = match axis {
                    wl_pointer::Axis::VerticalScroll => state.vertical_modifier,
                    wl_pointer::Axis::HorizontalScroll => state.horizontal_modifier,
                    _ => unreachable!(),
                };

                let scroll_delta = state.discrete_scroll_delta.get_or_insert(point(0.0, 0.0));
                let wheel_percent = value120 as f32 / 120.0;
                match axis {
                    wl_pointer::Axis::VerticalScroll => {
                        scroll_delta.y += wheel_percent * axis_modifier * SCROLL_LINES;
                    }
                    wl_pointer::Axis::HorizontalScroll => {
                        scroll_delta.x += wheel_percent * axis_modifier * SCROLL_LINES;
                    }
                    _ => unreachable!(),
                }
            }
            wl_pointer::Event::Frame => {
                if state.scroll_event_received {
                    state.scroll_event_received = false;
                    let continuous = state.continuous_scroll_delta.take();
                    let discrete = state.discrete_scroll_delta.take();
                    if let Some(continuous) = continuous {
                        if let Some(window) = state.mouse_focused_window.clone() {
                            let input = PlatformInput::ScrollWheel(ScrollWheelEvent {
                                position: state.mouse_location.unwrap(),
                                delta: ScrollDelta::Pixels(continuous),
                                modifiers: state.modifiers,
                                touch_phase: TouchPhase::Moved,
                            });
                            drop(state);
                            window.handle_input(input);
                        }
                    } else if let Some(discrete) = discrete
                        && let Some(window) = state.mouse_focused_window.clone()
                    {
                        let input = PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position: state.mouse_location.unwrap(),
                            delta: ScrollDelta::Lines(discrete),
                            modifiers: state.modifiers,
                            touch_phase: TouchPhase::Moved,
                        });
                        drop(state);
                        window.handle_input(input);
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_pointer_gestures_v1::ZwpPointerGesturesV1, ()> for WaylandClientStatePtr {
    fn event(
        _this: &mut Self,
        _: &zwp_pointer_gestures_v1::ZwpPointerGesturesV1,
        _: <zwp_pointer_gestures_v1::ZwpPointerGesturesV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // The gesture manager doesn't generate events
    }
}

impl Dispatch<zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _: &zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1,
        event: <zwp_pointer_gesture_pinch_v1::ZwpPointerGesturePinchV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use gpui::PinchEvent;

        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(window) = state.mouse_focused_window.clone() else {
            return;
        };

        match event {
            zwp_pointer_gesture_pinch_v1::Event::Begin {
                serial: _,
                time: _,
                surface: _,
                fingers: _,
            } => {
                state.pinch_scale = 1.0;
                let input = PlatformInput::Pinch(PinchEvent {
                    position: state.mouse_location.unwrap_or(point(px(0.0), px(0.0))),
                    delta: 0.0,
                    modifiers: state.modifiers,
                    phase: TouchPhase::Started,
                });
                drop(state);
                window.handle_input(input);
            }
            zwp_pointer_gesture_pinch_v1::Event::Update { time: _, scale, .. } => {
                let new_absolute_scale = scale as f32;
                let previous_scale = state.pinch_scale;
                let zoom_delta = new_absolute_scale - previous_scale;
                state.pinch_scale = new_absolute_scale;

                let input = PlatformInput::Pinch(PinchEvent {
                    position: state.mouse_location.unwrap_or(point(px(0.0), px(0.0))),
                    delta: zoom_delta,
                    modifiers: state.modifiers,
                    phase: TouchPhase::Moved,
                });
                drop(state);
                window.handle_input(input);
            }
            zwp_pointer_gesture_pinch_v1::Event::End {
                serial: _,
                time: _,
                cancelled: _,
            } => {
                state.pinch_scale = 1.0;
                let input = PlatformInput::Pinch(PinchEvent {
                    position: state.mouse_location.unwrap_or(point(px(0.0), px(0.0))),
                    delta: 0.0,
                    modifiers: state.modifiers,
                    phase: TouchPhase::Ended,
                });
                drop(state);
                window.handle_input(input);
            }
            _ => {}
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, ObjectId> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        event: <wp_fractional_scale_v1::WpFractionalScaleV1 as Proxy>::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        window.handle_fractional_scale_event(event);
    }
}

impl Dispatch<zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1, ObjectId>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _: &zxdg_toplevel_decoration_v1::ZxdgToplevelDecorationV1,
        event: zxdg_toplevel_decoration_v1::Event,
        surface_id: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();
        let Some(window) = get_window(&mut state, surface_id) else {
            return;
        };

        drop(state);
        window.handle_toplevel_decoration_event(event);
    }
}

impl Dispatch<wl_data_device::WlDataDevice, ()> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        _: &wl_data_device::WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            // Clipboard
            wl_data_device::Event::DataOffer { id: data_offer } => {
                state.data_offers.push(DataOffer::new(data_offer));
                if state.data_offers.len() > 2 {
                    // At most we store a clipboard offer and a drag and drop offer.
                    state.data_offers.remove(0).inner.destroy();
                }
            }
            wl_data_device::Event::Selection { id: data_offer } => {
                if let Some(offer) = data_offer {
                    let offer = state
                        .data_offers
                        .iter()
                        .find(|wrapper| wrapper.inner.id() == offer.id());
                    let offer = offer.cloned();
                    state.clipboard.set_offer(offer);
                } else {
                    state.clipboard.set_offer(None);
                }
            }

            // Drag and drop
            wl_data_device::Event::Enter {
                serial,
                surface,
                x,
                y,
                id: data_offer,
            } => {
                state.serial_tracker.update(SerialKind::DataDevice, serial);
                if let Some(data_offer) = data_offer {
                    let Some(drag_window) = get_window(&mut state, &surface.id()) else {
                        return;
                    };

                    let offer = state
                        .data_offers
                        .iter()
                        .position(|wrapper| wrapper.inner.id() == data_offer.id())
                        .map(|index| state.data_offers.remove(index));
                    let position = Point::new(x.into(), y.into());

                    let internal_session = offer
                        .as_ref()
                        .filter(|offer| offer.has_mime_type(&state.internal_drag_mime))
                        .and_then(|_| state.native_drag_source.as_ref())
                        .map(|source| source.session_id);
                    if let Some(session_id) = internal_session {
                        data_offer.accept(serial, Some(state.internal_drag_mime.clone()));
                        let files = state
                            .native_drag_source
                            .as_ref()
                            .and_then(|source| source.files.as_ref());
                        let actions = file_actions(files);
                        let preferred = files.map_or(DndAction::Move, |files| {
                            match files.options().preferred_action {
                                gpui::DragAction::Copy => DndAction::Copy,
                                _ => DndAction::Move,
                            }
                        });
                        data_offer.set_actions(actions, preferred);
                        state.drag.data_offer = Some(data_offer);
                        state.drag.kind = Some(DragOfferKind::Internal(session_id));
                        state.drag.window = Some(drag_window.clone());
                        state.drag.position = position;
                        drop(state);
                        drag_window.handle_input(PlatformInput::InternalDrag(
                            InternalDragEvent::Entered {
                                session_id,
                                position,
                            },
                        ));
                        return;
                    }

                    let accepts_files = offer
                        .as_ref()
                        .is_some_and(|offer| offer.has_mime_type(FILE_LIST_MIME_TYPE));
                    if !accepts_files {
                        data_offer.accept(serial, None);
                        data_offer.destroy();
                        return;
                    }

                    const ACTIONS: DndAction = DndAction::Copy;
                    data_offer.accept(serial, Some(FILE_LIST_MIME_TYPE.to_string()));
                    data_offer.set_actions(ACTIONS, ACTIONS);

                    let pipe = Pipe::new().unwrap();
                    data_offer.receive(FILE_LIST_MIME_TYPE.to_string(), unsafe {
                        BorrowedFd::borrow_raw(pipe.write.as_raw_fd())
                    });
                    let fd = pipe.read;
                    drop(pipe.write);

                    let read_task = state.common.background_executor.spawn(async {
                        let buffer = read_fd_with_timeout(fd, PIPE_READ_TIMEOUT)?;
                        let text = String::from_utf8(buffer)?;
                        anyhow::Ok(text)
                    });

                    let this = this.clone();
                    state
                        .common
                        .foreground_executor
                        .spawn(async move {
                            let file_list = match read_task.await {
                                Ok(list) => list,
                                Err(err) => {
                                    log::error!("error reading drag and drop pipe: {err:?}");
                                    return;
                                }
                            };

                            let paths: SmallVec<[_; 2]> = file_list
                                .lines()
                                .filter_map(|path| Url::parse(path).log_err())
                                .filter_map(|url| match url.to_file_path() {
                                    Ok(url) => Some(url),
                                    Err(()) => {
                                        log::error!("Failed turn {url:?} into a file path");
                                        None
                                    }
                                })
                                .collect();
                            // Prevent dropping text from other programs.
                            if paths.is_empty() {
                                data_offer.destroy();
                                return;
                            }

                            let input = PlatformInput::FileDrop(FileDropEvent::Entered {
                                position,
                                paths: gpui::ExternalPaths(paths),
                            });

                            let client = this.get_client();
                            let mut state = client.borrow_mut();
                            state.drag.data_offer = Some(data_offer);
                            state.drag.kind = Some(DragOfferKind::ExternalFiles);
                            state.drag.window = Some(drag_window.clone());
                            state.drag.position = position;

                            drop(state);
                            drag_window.handle_input(input);
                        })
                        .detach();
                }
            }
            wl_data_device::Event::Motion { x, y, .. } => {
                let Some(drag_window) = state.drag.window.clone() else {
                    return;
                };
                let position = Point::new(x.into(), y.into());
                state.drag.position = position;

                let input = match state.drag.kind {
                    Some(DragOfferKind::Internal(session_id)) => {
                        PlatformInput::InternalDrag(InternalDragEvent::Moved {
                            session_id,
                            position,
                        })
                    }
                    Some(DragOfferKind::ExternalFiles) => {
                        PlatformInput::FileDrop(FileDropEvent::Pending { position })
                    }
                    None => return,
                };
                drop(state);
                drag_window.handle_input(input);
            }
            wl_data_device::Event::Leave => {
                let Some(drag_window) = state.drag.window.clone() else {
                    return;
                };
                let kind = state.drag.kind.take();
                if let Some(data_offer) = state.drag.data_offer.take() {
                    data_offer.destroy();
                }

                state.drag.window = None;

                let input = match kind {
                    Some(DragOfferKind::Internal(session_id)) => {
                        PlatformInput::InternalDrag(InternalDragEvent::Left { session_id })
                    }
                    Some(DragOfferKind::ExternalFiles) => {
                        PlatformInput::FileDrop(FileDropEvent::Exited {})
                    }
                    None => return,
                };
                drop(state);
                drag_window.handle_input(input);
            }
            wl_data_device::Event::Drop => {
                let Some(drag_window) = state.drag.window.clone() else {
                    return;
                };
                let data_offer = state.drag.data_offer.take();
                let kind = state.drag.kind.take();
                let position = state.drag.position;
                state.drag.window = None;
                drop(state);

                match kind {
                    Some(DragOfferKind::Internal(session_id)) => {
                        let result = drag_window.handle_input(PlatformInput::InternalDrag(
                            InternalDragEvent::Dropped {
                                session_id,
                                position,
                            },
                        ));
                        if let Some(data_offer) = data_offer {
                            finish_data_offer(data_offer, result.drag_drop_accepted);
                        }
                    }
                    Some(DragOfferKind::ExternalFiles) => {
                        let result = drag_window.handle_input(PlatformInput::FileDrop(
                            FileDropEvent::Submit { position },
                        ));
                        if let Some(data_offer) = data_offer {
                            finish_data_offer(data_offer, result.drag_drop_accepted);
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    event_created_child!(WaylandClientStatePtr, wl_data_device::WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (wl_data_offer::WlDataOffer, WaylandDataOfferState::default()),
    ]);
}

impl Dispatch<wl_data_offer::WlDataOffer, WaylandDataOfferState> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        data_offer: &wl_data_offer::WlDataOffer,
        event: wl_data_offer::Event,
        data: &WaylandDataOfferState,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            wl_data_offer::Event::Offer { mime_type } => {
                // Clipboard
                if let Some(offer) = state
                    .data_offers
                    .iter_mut()
                    .find(|wrapper| wrapper.inner.id() == data_offer.id())
                {
                    offer.add_mime_type(mime_type);
                }
            }
            wl_data_offer::Event::Action { dnd_action } => {
                let action = match dnd_action {
                    WEnum::Value(action) if !action.is_empty() => Some(action),
                    WEnum::Value(_) | WEnum::Unknown(_) => None,
                };
                data.set_selected_action(action);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_data_source::WlDataSource, WaylandDataSourceKind> for WaylandClientStatePtr {
    fn event(
        this: &mut Self,
        data_source: &wl_data_source::WlDataSource,
        event: wl_data_source::Event,
        kind: &WaylandDataSourceKind,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match (kind, event) {
            (
                WaylandDataSourceKind::Clipboard(source),
                wl_data_source::Event::Send { mime_type, fd },
            ) => {
                state.clipboard.send(source, &mime_type, fd);
            }
            (WaylandDataSourceKind::Clipboard(_), wl_data_source::Event::Cancelled) => {
                data_source.destroy();
            }
            (
                WaylandDataSourceKind::InternalDrag(session_id),
                wl_data_source::Event::Send { fd, mime_type },
            ) => {
                let bytes = state
                    .native_drag_source
                    .as_ref()
                    .filter(|source| {
                        source.session_id == *session_id && mime_type == FILE_LIST_MIME_TYPE
                    })
                    .and_then(|source| source.files.as_ref())
                    .map(|files| files.uri_list().clone());
                drop(state);
                if let Some(bytes) = bytes {
                    if let Err(error) = this.send_drag_files(*session_id, fd, bytes) {
                        log::warn!("file drag transfer failed: {error:#}");
                        this.fail_drag(*session_id, gpui::DragFailure::Transfer);
                    }
                }
            }
            (
                WaylandDataSourceKind::InternalDrag(session_id),
                wl_data_source::Event::Target { mime_type },
            ) => {
                if let Some(source) = state
                    .native_drag_source
                    .as_mut()
                    .filter(|source| source.session_id == *session_id)
                {
                    if !source.drop_performed {
                        source.external_target = mime_type.as_deref() == Some(FILE_LIST_MIME_TYPE);
                    }
                }
            }
            (
                WaylandDataSourceKind::InternalDrag(session_id),
                wl_data_source::Event::Action { dnd_action },
            ) => {
                if let Some(source) = state
                    .native_drag_source
                    .as_mut()
                    .filter(|source| source.session_id == *session_id)
                {
                    source.action = match dnd_action {
                        WEnum::Value(DndAction::Copy) => Some(gpui::DragAction::Copy),
                        WEnum::Value(DndAction::Move) => Some(gpui::DragAction::Move),
                        _ => None,
                    };
                }
            }
            (
                WaylandDataSourceKind::InternalDrag(session_id),
                wl_data_source::Event::DndDropPerformed,
            ) => {
                let session_id = *session_id;
                if let Some(source) = state
                    .native_drag_source
                    .as_mut()
                    .filter(|source| source.session_id == session_id)
                {
                    source.drop_performed = true;
                }
                let Some(window) = state
                    .native_drag_source
                    .as_ref()
                    .filter(|source| source.session_id == session_id)
                    .map(|source| source.window.clone())
                else {
                    return;
                };
                drop(state);
                this.drag_timeout(session_id);
                window.handle_input(PlatformInput::InternalDrag(
                    InternalDragEvent::SourceDropPerformed { session_id },
                ));
            }
            (
                WaylandDataSourceKind::InternalDrag(session_id),
                wl_data_source::Event::DndFinished,
            ) => {
                let session_id = *session_id;
                let Some(source) = state
                    .native_drag_source
                    .as_ref()
                    .filter(|source| source.session_id == session_id)
                else {
                    return;
                };
                let external = source.external_target;
                let action = state
                    .native_drag_source
                    .as_ref()
                    .filter(|source| source.session_id == session_id && source.external_target)
                    .and_then(|source| {
                        source.action.filter(|action| {
                            source.files.as_ref().is_some_and(|files| {
                                files.options().allowed_actions.allows(*action)
                            })
                        })
                    });
                let window = state
                    .native_drag_source
                    .as_ref()
                    .filter(|source| source.session_id == session_id)
                    .map(|source| source.window.clone());
                if window.is_some() {
                    state.native_drag_source = None;
                }
                data_source.destroy();
                drop(state);
                if let Some(window) = window {
                    window.handle_input(PlatformInput::InternalDrag(
                        if external && action.is_none() {
                            InternalDragEvent::SourceFailed {
                                session_id,
                                failure: gpui::DragFailure::Protocol,
                            }
                        } else {
                            InternalDragEvent::SourceFinished { session_id, action }
                        },
                    ));
                }
            }
            (WaylandDataSourceKind::InternalDrag(session_id), wl_data_source::Event::Cancelled) => {
                let session_id = *session_id;
                if !state
                    .native_drag_source
                    .as_ref()
                    .is_some_and(|source| source.session_id == session_id)
                {
                    return;
                }
                let window = state
                    .native_drag_source
                    .as_ref()
                    .filter(|source| source.session_id == session_id)
                    .map(|source| source.window.clone());
                if window.is_some() {
                    state.native_drag_source = None;
                }
                data_source.destroy();
                drop(state);
                if let Some(window) = window {
                    window.handle_input(PlatformInput::InternalDrag(
                        InternalDragEvent::SourceCancelled { session_id },
                    ));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _: &zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1,
        event: zwp_primary_selection_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        match event {
            zwp_primary_selection_device_v1::Event::DataOffer { offer } => {
                let old_offer = state.primary_data_offer.replace(DataOffer::new(offer));
                if let Some(old_offer) = old_offer {
                    old_offer.inner.destroy();
                }
            }
            zwp_primary_selection_device_v1::Event::Selection { id: data_offer } => {
                if data_offer.is_some() {
                    let offer = state.primary_data_offer.clone();
                    state.clipboard.set_primary_offer(offer);
                } else {
                    state.clipboard.set_primary_offer(None);
                }
            }
            _ => {}
        }
    }

    event_created_child!(WaylandClientStatePtr, zwp_primary_selection_device_v1::ZwpPrimarySelectionDeviceV1, [
        zwp_primary_selection_device_v1::EVT_DATA_OFFER_OPCODE => (zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()),
    ]);
}

impl Dispatch<zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1, ()>
    for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        _data_offer: &zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1,
        event: zwp_primary_selection_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let mut state = client.borrow_mut();

        if let zwp_primary_selection_offer_v1::Event::Offer { mime_type } = event
            && let Some(offer) = state.primary_data_offer.as_mut()
        {
            offer.add_mime_type(mime_type);
        }
    }
}

impl
    Dispatch<
        zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        crate::linux::clipboard::ClipboardSource,
    > for WaylandClientStatePtr
{
    fn event(
        this: &mut Self,
        selection_source: &zwp_primary_selection_source_v1::ZwpPrimarySelectionSourceV1,
        event: zwp_primary_selection_source_v1::Event,
        source: &crate::linux::clipboard::ClipboardSource,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let client = this.get_client();
        let state = client.borrow_mut();

        match event {
            zwp_primary_selection_source_v1::Event::Send { mime_type, fd } => {
                state.clipboard.send(source, &mime_type, fd);
            }
            zwp_primary_selection_source_v1::Event::Cancelled => {
                selection_source.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<XdgWmDialogV1, ()> for WaylandClientStatePtr {
    fn event(
        _: &mut Self,
        _: &XdgWmDialogV1,
        _: <XdgWmDialogV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgDialogV1, ()> for WaylandClientStatePtr {
    fn event(
        _state: &mut Self,
        _proxy: &XdgDialogV1,
        _event: <XdgDialogV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qhandle: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a running Wayland compositor with an enabled output"]
    fn startup_displays_are_available_before_opening_a_window() {
        let connection = Connection::connect_to_env().unwrap();
        let client = WaylandClient::with_connection(connection, None, None);
        let displays = client.displays();
        assert!(
            !displays.is_empty(),
            "startup must receive the initial output batch"
        );
        assert!(client.0.borrow().windows.is_empty());
        for display in displays {
            let state = client.0.borrow();
            let output = state.output_for_display(display.id()).unwrap();
            assert!(state.outputs.contains_key(&output.id()));
            assert!(display.bounds().size.width > px(0.));
            assert!(display.bounds().size.height > px(0.));
        }
    }

    fn output_geometry(x: i32, y: i32, transform: wl_output::Transform) -> wl_output::Event {
        wl_output::Event::Geometry {
            x,
            y,
            physical_width: 300,
            physical_height: 190,
            subpixel: WEnum::Value(wl_output::Subpixel::HorizontalRgb),
            make: "Test".into(),
            model: "Display".into(),
            transform: WEnum::Value(transform),
        }
    }

    fn output_mode(flags: wl_output::Mode, width: i32, height: i32) -> wl_output::Event {
        wl_output::Event::Mode {
            flags: WEnum::Value(flags),
            width,
            height,
            refresh: 60000,
        }
    }

    fn fractional_output(version: Option<u32>) -> InProgressOutput {
        let mut pending = InProgressOutput {
            xdg_version: version,
            ..Default::default()
        };
        pending.apply(output_geometry(1707, 0, wl_output::Transform::Normal));
        pending.apply(output_mode(wl_output::Mode::Current, 3840, 2160));
        pending.apply(wl_output::Event::Scale { factor: 2 });
        pending
    }

    #[test]
    fn fractional_output_bounds_are_committed_at_wl_done() {
        let mut pending = fractional_output(Some(3));
        assert!(pending.apply(wl_output::Event::Done).is_none());
        pending.apply_xdg(zxdg_output_v1::Event::LogicalPosition { x: 1707, y: -120 });
        pending.apply_xdg(zxdg_output_v1::Event::LogicalSize {
            width: 2560,
            height: 1440,
        });
        assert!(pending.apply_xdg(zxdg_output_v1::Event::Done).is_none());
        assert!(pending.complete().is_none());
        let first = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(
            first.logical_bounds(),
            Bounds::new(point(px(1707.), px(-120.)), size(px(2560.), px(1440.)))
        );
        assert_eq!(first.display_scale_factor(), 1.5);
        assert_eq!(first.scale, 2, "surface buffer scale remains integer");

        // A scale/layout update need not repeat the physical mode or name.
        pending.apply_xdg(zxdg_output_v1::Event::LogicalPosition { x: -3072, y: 0 });
        pending.apply_xdg(zxdg_output_v1::Event::LogicalSize {
            width: 3072,
            height: 1728,
        });
        assert_eq!(pending.complete().unwrap(), first);
        let next = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(
            next.logical_bounds(),
            Bounds::new(point(px(-3072.), px(0.)), size(px(3072.), px(1728.)))
        );
        assert_eq!(next.display_scale_factor(), 1.25);
        assert_ne!(first, next);
    }

    #[test]
    fn legacy_xdg_output_waits_for_its_own_done() {
        for version in [1, 2] {
            let mut pending = fractional_output(Some(version));
            pending.apply_xdg(zxdg_output_v1::Event::LogicalPosition { x: 1707, y: 0 });
            pending.apply_xdg(zxdg_output_v1::Event::LogicalSize {
                width: 2560,
                height: 1440,
            });
            assert!(pending.apply(wl_output::Event::Done).is_none());
            let first = pending.apply_xdg(zxdg_output_v1::Event::Done).unwrap();
            assert_eq!(first.logical_bounds().size, size(px(2560.), px(1440.)));

            pending.apply_xdg(zxdg_output_v1::Event::LogicalPosition { x: -2560, y: 0 });
            assert_eq!(pending.apply(wl_output::Event::Done).unwrap(), first);
            let moved = pending.apply_xdg(zxdg_output_v1::Event::Done).unwrap();
            assert_eq!(moved.logical_bounds().origin, point(px(-2560.), px(0.)));
            assert_eq!(moved.logical_bounds().size, first.logical_bounds().size);
        }
    }

    #[test]
    fn rotated_output_uses_logical_size_without_rotating_it_again() {
        let mut pending = fractional_output(Some(3));
        pending.apply(output_geometry(-1440, 0, wl_output::Transform::_90));
        pending.apply_xdg(zxdg_output_v1::Event::LogicalPosition { x: -1440, y: 0 });
        pending.apply_xdg(zxdg_output_v1::Event::LogicalSize {
            width: 1440,
            height: 2560,
        });
        let output = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(
            output.bounds.size,
            size(DevicePixels(2160), DevicePixels(3840))
        );
        assert_eq!(
            output.logical_bounds(),
            Bounds::new(point(px(-1440.), px(0.)), size(px(1440.), px(2560.)))
        );
        assert_eq!(output.display_scale_factor(), 1.5);
    }

    #[test]
    fn wl_output_fallback_does_not_scale_global_position() {
        let mut pending = fractional_output(None);
        pending.apply(output_geometry(-1920, 120, wl_output::Transform::Normal));
        let output = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(
            output.logical_bounds(),
            Bounds::new(point(px(-1920.), px(120.)), size(px(1920.), px(1080.)))
        );
        assert_eq!(output.display_scale_factor(), 2.);
    }

    #[test]
    fn output_updates_are_batched_and_preserve_unchanged_properties() {
        let mut pending = InProgressOutput::default();
        assert!(
            pending
                .apply(output_geometry(0, 0, wl_output::Transform::Normal))
                .is_none()
        );
        assert!(
            pending
                .apply(output_mode(wl_output::Mode::Current, 2560, 1600))
                .is_none()
        );
        assert!(
            pending
                .apply(wl_output::Event::Name {
                    name: "eDP-1".into()
                })
                .is_none()
        );
        assert!(
            pending
                .apply(wl_output::Event::Scale { factor: 2 })
                .is_none()
        );
        let first = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(
            first.bounds.size,
            size(DevicePixels(2560), DevicePixels(1600))
        );
        // A later resume/configuration batch need not repeat the name or mode.
        assert!(
            pending
                .apply(output_geometry(1920, 0, wl_output::Transform::Normal))
                .is_none()
        );
        assert!(
            pending
                .apply(wl_output::Event::Scale { factor: 1 })
                .is_none()
        );
        let updated = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(updated.scale, 1);
        assert_eq!(updated.name.as_deref(), Some("eDP-1"));
        assert_eq!(
            updated.bounds.origin,
            point(DevicePixels(1920), DevicePixels(0))
        );
        assert_eq!(updated.bounds.size, first.bounds.size);
        assert_eq!(first.scale, 2);
        assert_eq!(first.bounds.origin, Point::default());
    }

    #[test]
    fn output_modes_use_current_not_last_advertised_and_handle_rotation() {
        let mut pending = InProgressOutput::default();
        pending.apply(output_geometry(0, 0, wl_output::Transform::_90));
        // Preferred and current are independent flags.
        pending.apply(output_mode(wl_output::Mode::Current, 2560, 1600));
        pending.apply(output_mode(wl_output::Mode::Preferred, 1920, 1080));
        let output = pending.apply(wl_output::Event::Done).unwrap();
        assert_eq!(
            output.bounds.size,
            size(DevicePixels(1600), DevicePixels(2560))
        );
        pending.apply(output_mode(wl_output::Mode::Current, 1920, 1200));
        assert_eq!(
            pending.apply(wl_output::Event::Done).unwrap().bounds.size,
            size(DevicePixels(1200), DevicePixels(1920))
        );
    }

    #[test]
    fn data_offer_finish_requires_acceptance_and_selected_action() {
        assert!(data_offer_can_finish(true, Some(DndAction::Move)));
        assert!(data_offer_can_finish(true, Some(DndAction::Copy)));
        assert!(!data_offer_can_finish(false, Some(DndAction::Move)));
        assert!(!data_offer_can_finish(true, Some(DndAction::empty())));
        assert!(!data_offer_can_finish(true, None));
    }
}
