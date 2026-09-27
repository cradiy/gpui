//! Desktop pixel sampling with a frozen-output overlay and a pointer-following loupe.

use anyhow::{Context, Result, anyhow, bail, ensure};
use futures::channel::oneshot;
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::{AsFd, AsRawFd},
        unix::fs::{FileExt, OpenOptionsExt},
    },
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle, WEnum, delegate_noop,
    protocol::{
        wl_buffer, wl_compositor, wl_keyboard, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
        wl_shm_pool, wl_subcompositor, wl_subsurface, wl_surface,
    },
};
use wayland_protocols::wp::{
    cursor_shape::v1::client::{wp_cursor_shape_device_v1, wp_cursor_shape_manager_v1},
    viewporter::client::{wp_viewport, wp_viewporter},
};
use wayland_protocols_wlr::{
    layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1},
    screencopy::v1::client::{zwlr_screencopy_frame_v1, zwlr_screencopy_manager_v1},
};

static ACTIVE: AtomicBool = AtomicBool::new(false);
mod loupe;
mod pixels;
use loupe::{LOUPE_H, LOUPE_W, Loupe};
use pixels::{normalize, pixel_position};

pub(crate) enum Outcome {
    Unsupported,
    Complete(Option<gpui::Rgba>),
}

pub(crate) fn pick(text: Arc<dyn gpui::PlatformTextSystem>) -> oneshot::Receiver<Result<Outcome>> {
    let (tx, rx) = oneshot::channel();
    if ACTIVE.swap(true, Ordering::AcqRel) {
        let _ = tx.send(Err(anyhow!("A desktop color sampler is already open")));
        return rx;
    }
    // A separate connection keeps the modal sampling session off the application event loop.
    let spawn = std::thread::Builder::new()
        .name("screen-color-picker".into())
        .spawn(move || {
            struct ActiveGuard;
            impl Drop for ActiveGuard {
                fn drop(&mut self) {
                    ACTIVE.store(false, Ordering::Release);
                }
            }
            let guard = ActiveGuard;
            let result = run(text, || tx.is_canceled());
            drop(guard);
            let _ = tx.send(result);
        });
    if let Err(error) = spawn {
        ACTIVE.store(false, Ordering::Release);
        let (tx, rx) = oneshot::channel();
        let _ = tx.send(Err(
            anyhow!(error).context("Could not start screen color sampler")
        ));
        return rx;
    }
    rx
}

fn run(text: Arc<dyn gpui::PlatformTextSystem>, cancelled: impl Fn() -> bool) -> Result<Outcome> {
    let conn = Connection::connect_to_env()?;
    let mut queue = conn.new_event_queue();
    let qh = queue.handle();
    let mut state = Picker {
        text: Some(text),
        ..Picker::default()
    };
    conn.display().get_registry(&qh, ());
    queue.roundtrip(&mut state)?;
    queue.roundtrip(&mut state)?;
    state.registry_ready = true;
    let (Some(capture), Some(_), Some(_), Some(_), Some(_), Some(_)) = (
        state.capture.clone(),
        &state.layers,
        &state.compositor,
        &state.subcompositor,
        &state.shm,
        &state.viewporter,
    ) else {
        return Ok(Outcome::Unsupported);
    };
    ensure!(
        !state.outputs.is_empty(),
        "No outputs are available for screen sampling"
    );
    for (i, output) in state.outputs.iter().enumerate() {
        capture.capture_output(0, &output.output, &qh, i);
    }
    let started = Instant::now();
    let mut opened = false;
    loop {
        if cancelled() {
            return Ok(Outcome::Complete(None));
        }
        queue.dispatch_pending(&mut state)?;
        if let Some(result) = state.result.take() {
            return result.map(Outcome::Complete);
        }
        if !opened && state.outputs.iter().all(|o| o.pixels.is_some()) {
            state.open(&qh)?;
            opened = true;
        }
        if state
            .outputs
            .iter()
            .any(|o| o.surface.is_none() || o.logical_size.0 == 0)
            && started.elapsed() > Duration::from_secs(10)
        {
            bail!("Desktop color sampler timed out while preparing outputs");
        }
        if state.dirty {
            state.paint(&qh)?;
        }
        conn.flush()?;
        if let Some(guard) = queue.prepare_read() {
            let mut fd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: fd points to one initialized pollfd for the duration of poll.
            let ready = unsafe { libc::poll(&mut fd, 1, 50) };
            if ready < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(error.into());
                }
            } else if ready > 0 {
                guard.read()?;
            }
        }
    }
}

struct Buffer {
    file: File,
    buffer: wl_buffer::WlBuffer,
    len: usize,
    busy: bool,
}
impl Buffer {
    fn new(
        shm: &wl_shm::WlShm,
        width: u32,
        height: u32,
        stride: u32,
        format: wl_shm::Format,
        qh: &QueueHandle<Picker>,
        owner: Option<(usize, usize)>,
    ) -> Result<Self> {
        ensure!(
            width > 0
                && height > 0
                && stride >= width.checked_mul(4).context("Invalid capture width")?,
            "Invalid screen buffer size"
        );
        let len = stride
            .checked_mul(height)
            .filter(|v| *v <= i32::MAX as u32)
            .context("Screen buffer is too large")? as usize;
        let path = std::env::temp_dir().join(format!("gpui-color-{}", uuid::Uuid::new_v4()));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        std::fs::remove_file(path)?;
        file.set_len(len as u64)?;
        let pool = shm.create_pool(file.as_fd(), len as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            qh,
            owner,
        );
        pool.destroy();
        Ok(Self {
            file,
            buffer,
            len,
            busy: false,
        })
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        self.buffer.destroy();
    }
}

struct Output {
    name: u32,
    output: wl_output::WlOutput,
    transform: wl_output::Transform,
    capture: Option<Buffer>,
    format: wl_shm::Format,
    size: (u32, u32, u32),
    inverted: bool,
    pixels: Option<Vec<u32>>,
    surface: Option<wl_surface::WlSurface>,
    logical_size: (u32, u32),
    loupe: Option<(wl_surface::WlSurface, wl_subsurface::WlSubsurface)>,
    loupe_buffers: Vec<Buffer>,
    renderer: Option<Loupe>,
    background: Option<Buffer>,
}
#[derive(Default)]
struct Picker {
    text: Option<Arc<dyn gpui::PlatformTextSystem>>,
    registry_ready: bool,
    compositor: Option<wl_compositor::WlCompositor>,
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    shm: Option<wl_shm::WlShm>,
    layers: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    capture: Option<zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1>,
    viewporter: Option<wp_viewporter::WpViewporter>,
    cursor_manager: Option<wp_cursor_shape_manager_v1::WpCursorShapeManagerV1>,
    cursor: Option<wp_cursor_shape_device_v1::WpCursorShapeDeviceV1>,
    outputs: Vec<Output>,
    pointer: Option<(usize, f64, f64)>,
    dirty: bool,
    result: Option<Result<Option<gpui::Rgba>>>,
}
impl Picker {
    fn open(&mut self, qh: &QueueHandle<Self>) -> Result<()> {
        for (i, output) in self.outputs.iter_mut().enumerate() {
            let surface = self.compositor.as_ref().unwrap().create_surface(qh, ());
            let layer = self.layers.as_ref().unwrap().get_layer_surface(
                &surface,
                Some(&output.output),
                zwlr_layer_shell_v1::Layer::Overlay,
                "gpui-screen-color-picker".into(),
                qh,
                i,
            );
            layer.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top
                    | zwlr_layer_surface_v1::Anchor::Bottom
                    | zwlr_layer_surface_v1::Anchor::Left
                    | zwlr_layer_surface_v1::Anchor::Right,
            );
            layer.set_size(0, 0);
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(
                zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
            );
            let loupe = self.compositor.as_ref().unwrap().create_surface(qh, ());
            let region = self.compositor.as_ref().unwrap().create_region(qh, ());
            loupe.set_input_region(Some(&region));
            region.destroy();
            let sub = self
                .subcompositor
                .as_ref()
                .unwrap()
                .get_subsurface(&loupe, &surface, qh, ());
            output.loupe = Some((loupe, sub));
            surface.commit();
            output.surface = Some(surface);
        }
        Ok(())
    }
    fn sample(&self) -> Option<u32> {
        let (i, x, y) = self.pointer?;
        let o = &self.outputs[i];
        let (px, py) = pixel_position(x, y, o.logical_size, (o.size.0, o.size.1))?;
        Some(o.pixels.as_ref()?[(py * o.size.0 + px) as usize])
    }
    fn paint(&mut self, _: &QueueHandle<Self>) -> Result<()> {
        let Some((i, x, y)) = self.pointer else {
            self.dirty = false;
            return Ok(());
        };
        let o = &mut self.outputs[i];
        let Some((px, py)) = pixel_position(x, y, o.logical_size, (o.size.0, o.size.1)) else {
            return Ok(());
        };
        let (surface, sub) = o.loupe.as_ref().unwrap();
        // Keep the visible panel 8 logical pixels from the pointer; the buffer
        // includes 10 pixels of transparent shadow padding on each side.
        let place = |pos: f64, size: u32, extent: usize| {
            let p = if pos + extent as f64 - 2.0 <= size as f64 {
                pos - 2.0
            } else {
                pos + 2.0 - extent as f64
            };
            p.round()
                .max(0.0)
                .min((size as f64 - extent as f64).max(0.0)) as i32
        };
        sub.set_position(
            place(x, o.logical_size.0, LOUPE_W),
            place(y, o.logical_size.1, LOUPE_H),
        );
        let Some(buffer) = o.loupe_buffers.iter_mut().find(|b| !b.busy) else {
            // Position can follow the pointer even while both pixel buffers
            // are held by the compositor. Repaint with the newest sample on release.
            o.surface.as_ref().unwrap().commit();
            return Ok(());
        };
        let renderer = o.renderer.as_ref().unwrap();
        let pixels = renderer.paint(o.pixels.as_ref().unwrap(), o.size.0, o.size.1, px, py);
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_ne_bytes()).collect();
        buffer.file.write_all_at(&bytes, 0)?;
        buffer.busy = true;
        surface.attach(Some(&buffer.buffer), 0, 0);
        surface.damage_buffer(
            0,
            0,
            (LOUPE_W * renderer.scale) as i32,
            (LOUPE_H * renderer.scale) as i32,
        );
        surface.commit();
        o.surface.as_ref().unwrap().commit();
        self.dirty = false;
        Ok(())
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Picker {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match interface.as_str() {
                "wl_compositor" if version >= 4 => {
                    s.compositor = Some(registry.bind(name, 4, qh, ()))
                }
                "wl_subcompositor" => s.subcompositor = Some(registry.bind(name, 1, qh, ())),
                "wl_shm" => s.shm = Some(registry.bind(name, 1, qh, ())),
                "zwlr_layer_shell_v1" => s.layers = Some(registry.bind(name, 1, qh, ())),
                // Version 1 guarantees SHM and does not require buffer_done before copy.
                "zwlr_screencopy_manager_v1" => s.capture = Some(registry.bind(name, 1, qh, ())),
                "wp_viewporter" => s.viewporter = Some(registry.bind(name, 1, qh, ())),
                "wp_cursor_shape_manager_v1" => {
                    s.cursor_manager = Some(registry.bind(name, 1, qh, ()))
                }
                "wl_seat" => {
                    registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(5), qh, ());
                }
                "wl_output" => {
                    if s.registry_ready {
                        s.result = Some(Ok(None));
                        return;
                    }
                    let output = registry.bind(name, version.min(3), qh, s.outputs.len());
                    s.outputs.push(Output {
                        name,
                        output,
                        transform: wl_output::Transform::Normal,
                        capture: None,
                        format: wl_shm::Format::Argb8888,
                        size: (0, 0, 0),
                        inverted: false,
                        pixels: None,
                        surface: None,
                        logical_size: (0, 0),
                        loupe: None,
                        loupe_buffers: Vec::new(),
                        renderer: None,
                        background: None,
                    });
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name }
                if s.outputs.iter().any(|o| o.name == name) =>
            {
                s.result = Some(Ok(None))
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_output::WlOutput, usize> for Picker {
    fn event(
        s: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        i: &usize,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Geometry {
            transform: WEnum::Value(transform),
            ..
        } = event
        {
            if s.outputs[*i].surface.is_some() && s.outputs[*i].transform != transform {
                s.result = Some(Ok(None));
            }
            s.outputs[*i].transform = transform;
        }
    }
}
impl Dispatch<zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1, usize> for Picker {
    fn event(
        s: &mut Self,
        frame: &zwlr_screencopy_frame_v1::ZwlrScreencopyFrameV1,
        event: zwlr_screencopy_frame_v1::Event,
        i: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let result = (|| -> Result<()> {
            let o = &mut s.outputs[*i];
            match event {
                zwlr_screencopy_frame_v1::Event::Buffer {
                    format: WEnum::Value(format),
                    width,
                    height,
                    stride,
                } => {
                    ensure!(
                        matches!(
                            format,
                            wl_shm::Format::Argb8888
                                | wl_shm::Format::Xrgb8888
                                | wl_shm::Format::Abgr8888
                                | wl_shm::Format::Xbgr8888
                                | wl_shm::Format::Argb2101010
                                | wl_shm::Format::Xrgb2101010
                                | wl_shm::Format::Abgr2101010
                                | wl_shm::Format::Xbgr2101010
                        ),
                        "Unsupported screen pixel format: {format:?}"
                    );
                    let buffer = Buffer::new(
                        s.shm.as_ref().unwrap(),
                        width,
                        height,
                        stride,
                        format,
                        qh,
                        None,
                    )?;
                    frame.copy(&buffer.buffer);
                    o.capture = Some(buffer);
                    o.format = format;
                    o.size = (width, height, stride);
                }
                zwlr_screencopy_frame_v1::Event::Flags {
                    flags: WEnum::Value(flags),
                } => o.inverted = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
                zwlr_screencopy_frame_v1::Event::Ready { .. } => {
                    frame.destroy();
                    let capture = o
                        .capture
                        .take()
                        .context("Screen capture returned no buffer")?;
                    let mut bytes = vec![0; capture.len];
                    capture.file.read_exact_at(&mut bytes, 0)?;
                    let (pixels, width, height) =
                        normalize(&bytes, o.size, o.format, o.inverted, o.transform);
                    o.pixels = Some(pixels);
                    o.size = (width, height, width * 4);
                }
                zwlr_screencopy_frame_v1::Event::Failed => {
                    frame.destroy();
                    bail!("The compositor could not capture the screen");
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = result {
            s.result = Some(Err(e));
        }
    }
}
impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, usize> for Picker {
    fn event(
        s: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        i: &usize,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                let o = &mut s.outputs[*i];
                if o.logical_size != (0, 0) {
                    if o.logical_size != (width, height) {
                        s.result = Some(Ok(None));
                    }
                    return;
                }
                o.logical_size = (width, height);
                let result = (|| -> Result<()> {
                    ensure!(width > 0 && height > 0, "Empty screen overlay");
                    let scale = (o.size.0 as f64 / width as f64)
                        .max(o.size.1 as f64 / height as f64)
                        .ceil()
                        .clamp(1., 4.) as usize;
                    o.renderer = Some(Loupe::new(s.text.as_ref().unwrap().as_ref(), scale)?);
                    o.loupe.as_ref().unwrap().0.set_buffer_scale(scale as i32);
                    for b in 0..2 {
                        o.loupe_buffers.push(Buffer::new(
                            s.shm.as_ref().unwrap(),
                            (LOUPE_W * scale) as u32,
                            (LOUPE_H * scale) as u32,
                            (LOUPE_W * scale * 4) as u32,
                            wl_shm::Format::Argb8888,
                            qh,
                            Some((*i, b)),
                        )?);
                    }
                    let background = Buffer::new(
                        s.shm.as_ref().unwrap(),
                        o.size.0,
                        o.size.1,
                        o.size.2,
                        wl_shm::Format::Argb8888,
                        qh,
                        None,
                    )?;
                    let bytes: Vec<u8> = o
                        .pixels
                        .as_ref()
                        .unwrap()
                        .iter()
                        .flat_map(|p| p.to_ne_bytes())
                        .collect();
                    background.file.write_all_at(&bytes, 0)?;
                    let surface = o.surface.as_ref().unwrap();
                    let viewport = s.viewporter.as_ref().unwrap().get_viewport(surface, qh, ());
                    viewport.set_destination(width as i32, height as i32);
                    surface.attach(Some(&background.buffer), 0, 0);
                    surface.damage_buffer(0, 0, o.size.0 as i32, o.size.1 as i32);
                    surface.commit();
                    o.background = Some(background);
                    Ok(())
                })();
                if let Err(e) = result {
                    s.result = Some(Err(e));
                }
            }
            zwlr_layer_surface_v1::Event::Closed => s.result = Some(Ok(None)),
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for Picker {
    fn event(
        s: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(c),
        } = event
        {
            if c.contains(wl_seat::Capability::Pointer) {
                let p = seat.get_pointer(qh, ());
                if let Some(m) = &s.cursor_manager {
                    s.cursor = Some(m.get_pointer(&p, qh, ()));
                }
            }
            if c.contains(wl_seat::Capability::Keyboard) {
                seat.get_keyboard(qh, ());
            }
        }
    }
}
impl Dispatch<wl_pointer::WlPointer, ()> for Picker {
    fn event(
        s: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                serial,
                surface,
                surface_x,
                surface_y,
            } => {
                if let Some(i) = s
                    .outputs
                    .iter()
                    .position(|o| o.surface.as_ref() == Some(&surface))
                {
                    s.pointer = Some((i, surface_x, surface_y));
                    s.dirty = true;
                    if let Some(cursor) = &s.cursor {
                        cursor.set_shape(serial, wp_cursor_shape_device_v1::Shape::Crosshair);
                    }
                }
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                if let Some((i, _, _)) = s.pointer {
                    s.pointer = Some((i, surface_x, surface_y));
                    s.dirty = true;
                }
            }
            wl_pointer::Event::Leave { .. } => {
                if let Some((i, _, _)) = s.pointer.take() {
                    if let Some((surface, _)) = &s.outputs[i].loupe {
                        surface.attach(None, 0, 0);
                        surface.commit();
                        s.outputs[i].surface.as_ref().unwrap().commit();
                    }
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => {
                if button == 0x110 {
                    if let Some(p) = s.sample() {
                        s.result = Some(Ok(Some(gpui::Rgba {
                            r: ((p >> 16) & 255) as f32 / 255.,
                            g: ((p >> 8) & 255) as f32 / 255.,
                            b: (p & 255) as f32 / 255.,
                            a: 1.,
                        })));
                    }
                } else if button == 0x111 {
                    s.result = Some(Ok(None));
                }
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_keyboard::WlKeyboard, ()> for Picker {
    fn event(
        s: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key: 1,
            state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            ..
        } = event
        {
            s.result = Some(Ok(None));
        }
    }
}
impl Dispatch<wl_buffer::WlBuffer, Option<(usize, usize)>> for Picker {
    fn event(
        s: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        owner: &Option<(usize, usize)>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let Some((i, b)) = owner {
            s.outputs[*i].loupe_buffers[*b].busy = false;
        }
    }
}
delegate_noop!(Picker: ignore wl_compositor::WlCompositor);
delegate_noop!(Picker: ignore wl_subcompositor::WlSubcompositor);
delegate_noop!(Picker: ignore wl_subsurface::WlSubsurface);
delegate_noop!(Picker: ignore wl_surface::WlSurface);
delegate_noop!(Picker: ignore wayland_client::protocol::wl_region::WlRegion);
delegate_noop!(Picker: ignore wl_shm::WlShm);
delegate_noop!(Picker: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Picker: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);
delegate_noop!(Picker: ignore zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1);
delegate_noop!(Picker: ignore wp_viewporter::WpViewporter);
delegate_noop!(Picker: ignore wp_viewport::WpViewport);
delegate_noop!(Picker: ignore wp_cursor_shape_manager_v1::WpCursorShapeManagerV1);
delegate_noop!(Picker: ignore wp_cursor_shape_device_v1::WpCursorShapeDeviceV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "opens a desktop sampler on the live Wayland session for up to 8 seconds"]
    fn desktop_loupe_smoke() {
        let start = Instant::now();
        let text = Arc::new(gpui_wgpu::CosmicTextSystem::new("sans-serif"));
        let result = run(text, || start.elapsed() > Duration::from_secs(8)).unwrap();
        assert!(
            matches!(result, Outcome::Complete(_)),
            "Desktop does not support the loupe protocols"
        );
    }
}
