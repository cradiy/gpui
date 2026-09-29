fn main() {
    example::main();
}

mod example {
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use async_channel::TrySendError;
    use gpui::{
        App, Bounds, Context, DevicePixels, ObjectFit, Render, ScreenCaptureFrame,
        ScreenCaptureStream, SurfaceFrame, SurfaceHandle, Task, Window, WindowBounds,
        WindowOptions, div, prelude::*, px, rgb, size, surface,
    };
    use gpui_platform::application;
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    use scap::frame::Frame;

    enum CaptureUpdate {
        Frame(Arc<SurfaceFrame>),
        Error(String),
    }

    struct ScreenCaptureExample {
        frame: Option<Arc<SurfaceFrame>>,
        frame_count: u64,
        status: String,
        _stream: Option<Box<dyn ScreenCaptureStream>>,
        _capture_task: Task<()>,
    }

    impl ScreenCaptureExample {
        fn new(cx: &mut Context<Self>) -> Self {
            let sources_rx = cx.screen_capture_sources();
            let foreground_executor = cx.foreground_executor().clone();
            let surface_handle = SurfaceHandle::new();
            let capture_task = cx.spawn(async move |this, cx| {
                let sources = match sources_rx.await {
                    Ok(Ok(sources)) => sources,
                    Ok(Err(error)) => {
                        Self::set_error(&this, cx, format!("Screen selection failed: {error:#}"));
                        return;
                    }
                    Err(error) => {
                        Self::set_error(
                            &this,
                            cx,
                            format!("Screen selection was canceled: {error}"),
                        );
                        return;
                    }
                };

                let Some(source) = sources.into_iter().next() else {
                    Self::set_error(&this, cx, "No capture source was selected.".to_string());
                    return;
                };
                let metadata = match source.metadata() {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        Self::set_error(
                            &this,
                            cx,
                            format!("Failed to read source metadata: {error:#}"),
                        );
                        return;
                    }
                };

                let (updates_tx, updates_rx) = async_channel::bounded(1);
                let stale_updates_rx = updates_rx.clone();
                let callback_handle = surface_handle.clone();
                let next_sequence = AtomicU64::new(1);
                let stream_rx = source.stream(
                    &foreground_executor,
                    Box::new(move |frame| {
                        let sequence = next_sequence.fetch_add(1, Ordering::Relaxed);
                        let update =
                            match to_surface_frame(frame, callback_handle.clone(), sequence) {
                                Ok(frame) => CaptureUpdate::Frame(Arc::new(frame)),
                                Err(error) => CaptureUpdate::Error(error),
                            };

                        if let Err(TrySendError::Full(update)) = updates_tx.try_send(update) {
                            stale_updates_rx.try_recv().ok();
                            updates_tx.try_send(update).ok();
                        }
                    }),
                );

                let stream = match stream_rx.await {
                    Ok(Ok(stream)) => stream,
                    Ok(Err(error)) => {
                        Self::set_error(&this, cx, format!("Failed to start capture: {error:#}"));
                        return;
                    }
                    Err(error) => {
                        Self::set_error(
                            &this,
                            cx,
                            format!("Capture startup was canceled: {error}"),
                        );
                        return;
                    }
                };

                let label = metadata
                    .label
                    .as_deref()
                    .unwrap_or("Portal selection")
                    .to_string();
                if this
                    .update(cx, |this, cx| {
                        this.status = format!(
                            "Streaming {label} · {} × {}",
                            metadata.resolution.width.0, metadata.resolution.height.0,
                        );
                        this._stream = Some(stream);
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }

                while let Ok(update) = updates_rx.recv().await {
                    if this
                        .update(cx, |this, cx| {
                            match update {
                                CaptureUpdate::Frame(frame) => {
                                    this.frame = Some(frame);
                                    this.frame_count += 1;
                                }
                                CaptureUpdate::Error(error) => {
                                    this.status = format!("Frame conversion failed: {error}");
                                }
                            }
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });

            Self {
                frame: None,
                frame_count: 0,
                status: "Choose a screen in the system portal.".to_string(),
                _stream: None,
                _capture_task: capture_task,
            }
        }

        fn set_error(this: &gpui::WeakEntity<Self>, cx: &mut gpui::AsyncApp, error: String) {
            this.update(cx, |this, cx| {
                this.status = error;
                cx.notify();
            })
            .ok();
        }
    }

    impl Render for ScreenCaptureExample {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let mut preview = div()
                .flex_1()
                .min_h_0()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .rounded_lg()
                .bg(rgb(0x090b0f));

            if let Some(frame) = self.frame.clone() {
                preview = preview.child(surface(frame).size_full().object_fit(ObjectFit::Contain));
            } else {
                preview = preview.child("Waiting for the first frame…");
            }

            div()
                .size_full()
                .flex()
                .flex_col()
                .gap_4()
                .p_5()
                .bg(rgb(0x15191f))
                .text_color(rgb(0xe7ebf0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(self.status.clone())
                        .child(format!("Frames presented: {}", self.frame_count)),
                )
                .child(preview)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    fn to_surface_frame(
        frame: ScreenCaptureFrame,
        handle: SurfaceHandle,
        sequence: u64,
    ) -> Result<SurfaceFrame, String> {
        match frame.0 {
            Frame::YUVFrame(frame) => {
                let frame_size = frame_size(frame.width, frame.height)?;
                let y_stride = positive_stride(frame.luminance_stride)?;
                let uv_stride = positive_stride(frame.chrominance_stride)?;
                SurfaceFrame::nv12(
                    handle,
                    sequence,
                    frame_size,
                    frame.luminance_bytes,
                    y_stride,
                    frame.chrominance_bytes,
                    uv_stride,
                    gpui::SurfaceColorInfo {
                        matrix: gpui::YuvMatrix::Bt709,
                        range: gpui::ColorRange::Limited,
                    },
                )
                .map_err(|error| error.to_string())
            }
            Frame::RGB(frame) => {
                let frame_size = frame_size(frame.width, frame.height)?;
                let mut rgba = Vec::with_capacity(frame.data.len() / 3 * 4);
                for pixel in frame.data.chunks_exact(3) {
                    rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
                SurfaceFrame::rgba(
                    handle,
                    sequence,
                    frame_size,
                    rgba,
                    packed_stride(frame.width, 4)?,
                )
                .map_err(|error| error.to_string())
            }
            Frame::RGBx(frame) => SurfaceFrame::rgba(
                handle,
                sequence,
                frame_size(frame.width, frame.height)?,
                frame.data,
                packed_stride(frame.width, 4)?,
            )
            .map_err(|error| error.to_string()),
            Frame::XBGR(mut frame) => {
                for pixel in frame.data.chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[pixel[1], pixel[2], pixel[3], 255]);
                }
                SurfaceFrame::bgra(
                    handle,
                    sequence,
                    frame_size(frame.width, frame.height)?,
                    frame.data,
                    packed_stride(frame.width, 4)?,
                )
                .map_err(|error| error.to_string())
            }
            Frame::BGRx(frame) => SurfaceFrame::bgra(
                handle,
                sequence,
                frame_size(frame.width, frame.height)?,
                frame.data,
                packed_stride(frame.width, 4)?,
            )
            .map_err(|error| error.to_string()),
            Frame::BGR0(frame) => SurfaceFrame::bgra(
                handle,
                sequence,
                frame_size(frame.width, frame.height)?,
                frame.data,
                packed_stride(frame.width, 4)?,
            )
            .map_err(|error| error.to_string()),
            Frame::BGRA(frame) => SurfaceFrame::bgra(
                handle,
                sequence,
                frame_size(frame.width, frame.height)?,
                frame.data,
                packed_stride(frame.width, 4)?,
            )
            .map_err(|error| error.to_string()),
        }
    }

    #[cfg(target_os = "macos")]
    fn to_surface_frame(
        frame: ScreenCaptureFrame,
        handle: SurfaceHandle,
        sequence: u64,
    ) -> Result<SurfaceFrame, String> {
        let pixel_buffer = frame.0;
        let frame_size = size(
            DevicePixels(
                i32::try_from(pixel_buffer.get_width())
                    .map_err(|_| "screen width does not fit in DevicePixels".to_string())?,
            ),
            DevicePixels(
                i32::try_from(pixel_buffer.get_height())
                    .map_err(|_| "screen height does not fit in DevicePixels".to_string())?,
            ),
        );
        SurfaceFrame::from_core_video(
            handle,
            sequence,
            Bounds {
                origin: Default::default(),
                size: frame_size,
            },
            frame_size,
            gpui::SurfaceFormat::Bgra8,
            unsafe { gpui::CoreVideoHandle::new(pixel_buffer) },
            gpui::SurfaceColorInfo::default(),
        )
        .map_err(|error| error.to_string())
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_os = "macos"
    )))]
    fn to_surface_frame(
        _frame: ScreenCaptureFrame,
        _handle: SurfaceHandle,
        _sequence: u64,
    ) -> Result<SurfaceFrame, String> {
        Err("screen capture preview is unsupported on this platform".to_string())
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    fn frame_size(width: i32, height: i32) -> Result<gpui::Size<DevicePixels>, String> {
        if width <= 0 || height <= 0 {
            return Err(format!("invalid frame size {width} × {height}"));
        }
        Ok(size(DevicePixels(width), DevicePixels(height)))
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    fn positive_stride(stride: i32) -> Result<u32, String> {
        u32::try_from(stride).map_err(|_| format!("invalid frame stride {stride}"))
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "windows"))]
    fn packed_stride(width: i32, bytes_per_pixel: u32) -> Result<u32, String> {
        u32::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(bytes_per_pixel))
            .ok_or_else(|| format!("invalid frame width {width}"))
    }

    pub fn main() {
        application().run(|cx: &mut App| {
            let bounds = Bounds::centered(None, size(px(1200.0), px(760.0)), cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_, cx| cx.new(ScreenCaptureExample::new),
            )
            .unwrap();
            cx.activate(true);
        });
    }
}
