//! Screen capture through scap, with native CoreVideo frames on macOS.
use crate::{
    DevicePixels, ForegroundExecutor, ScreenCaptureFrame, ScreenCaptureSource, ScreenCaptureStream,
    Size, SourceMetadata, size,
};
use anyhow::{Context as _, Result, anyhow};
use futures::channel::oneshot;
use scap::Target;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{self, AtomicBool};

/// Populates the receiver with the screens that can be captured.
///
/// [`start_scap_default_target_source`] should be used instead on Wayland, since
/// `scap_screen_sources` won't return any results.
#[allow(dead_code)]
pub fn scap_screen_sources(
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
    let (sources_tx, sources_rx) = oneshot::channel();
    get_screen_targets(sources_tx);
    to_dyn_screen_capture_sources(sources_rx, foreground_executor)
}

/// Starts screen capture for the default target, and populates the receiver with a single source
/// for it. The first frame of the screen capture is used to determine the size of the stream.
///
/// On Wayland (Linux), prompts the user to select a target, and populates the receiver with a
/// single screen capture source for their selection.
#[allow(dead_code)]
#[doc(hidden)]
pub fn start_scap_default_target_source(
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
    let (sources_tx, sources_rx) = oneshot::channel();
    start_default_target_screen_capture(sources_tx);
    to_dyn_screen_capture_sources(sources_rx, foreground_executor)
}

struct ScapCaptureSource {
    target: scap::Display,
    size: Size<DevicePixels>,
}

/// Populates the sender with the screens available for capture.
fn get_screen_targets(sources_tx: oneshot::Sender<Result<Vec<ScapCaptureSource>>>) {
    // Due to use of blocking APIs, a new thread is used.
    std::thread::spawn(|| {
        let targets = match scap::get_all_targets() {
            Ok(targets) => targets,
            Err(err) => {
                sources_tx.send(Err(err)).ok();
                return;
            }
        };
        let sources = targets
            .into_iter()
            .filter_map(|target| match target {
                scap::Target::Display(display) => {
                    let size = display_size(&display);
                    Some(ScapCaptureSource {
                        target: display,
                        size,
                    })
                }
                scap::Target::Window(_) => None,
            })
            .collect::<Vec<_>>();
        sources_tx.send(Ok(sources)).ok();
    });
}

impl ScreenCaptureSource for ScapCaptureSource {
    fn metadata(&self) -> Result<SourceMetadata> {
        Ok(metadata_for_target(
            Some(&Target::Display(self.target.clone())),
            self.size,
        ))
    }

    fn stream(
        &self,
        foreground_executor: &ForegroundExecutor,
        frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    ) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>> {
        let (stream_tx, stream_rx) = oneshot::channel();
        let target = self.target.clone();

        // Due to use of blocking APIs, a dedicated thread is used.
        std::thread::spawn(move || {
            match new_scap_capturer(Some(scap::Target::Display(target.clone()))) {
                Ok(mut capturer) => {
                    capturer.start_capture();
                    let resolution = display_size(&target);
                    let metadata = metadata_for_target(Some(&Target::Display(target)), resolution);
                    run_capture(capturer, metadata, frame_callback, stream_tx);
                }
                Err(e) => {
                    stream_tx.send(Err(e)).ok();
                }
            }
        });

        to_dyn_screen_capture_stream(stream_rx, foreground_executor)
    }
}

struct ScapDefaultTargetCaptureSource {
    // Sender populated by single call to `ScreenCaptureSource::stream`.
    stream_call_tx: std::sync::mpsc::SyncSender<(
        // Provides the result of `ScreenCaptureSource::stream`.
        oneshot::Sender<Result<ScapStream>>,
        // Callback for frames.
        Box<dyn Fn(ScreenCaptureFrame) + Send>,
    )>,
    metadata: SourceMetadata,
}

/// Starts screen capture on the default capture target, and populates the sender with the source.
fn start_default_target_screen_capture(
    sources_tx: oneshot::Sender<Result<Vec<ScapDefaultTargetCaptureSource>>>,
) {
    // Due to use of blocking APIs, a dedicated thread is used.
    std::thread::spawn(|| {
        let start_result = gpui_util::maybe!({
            let mut capturer = new_scap_capturer(None)?;
            capturer.start_capture();
            let first_frame = loop {
                match next_capture_frame(&capturer)
                    .context("Failed to get first frame of screenshare to get the size.")
                {
                    Ok(Some(frame)) => break frame,
                    Ok(None) => continue,
                    Err(error) => {
                        capturer.stop_capture();
                        return Err(error);
                    }
                }
            };
            let size = frame_size(&first_frame);
            let metadata = metadata_for_target(capturer.target(), size);
            Ok((capturer, metadata))
        });

        match start_result {
            Ok((mut capturer, metadata)) => {
                let (stream_call_tx, stream_rx) = std::sync::mpsc::sync_channel(1);
                sources_tx
                    .send(Ok(vec![ScapDefaultTargetCaptureSource {
                        stream_call_tx,
                        metadata: metadata.clone(),
                    }]))
                    .ok();
                let Ok((stream_tx, frame_callback)) = stream_rx.recv() else {
                    capturer.stop_capture();
                    return;
                };
                run_capture(capturer, metadata, frame_callback, stream_tx);
            }
            Err(e) => {
                sources_tx.send(Err(e)).ok();
            }
        }
    });
}

impl ScreenCaptureSource for ScapDefaultTargetCaptureSource {
    fn metadata(&self) -> Result<SourceMetadata> {
        Ok(self.metadata.clone())
    }

    fn stream(
        &self,
        foreground_executor: &ForegroundExecutor,
        frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    ) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>> {
        let (tx, rx) = oneshot::channel();
        match self.stream_call_tx.try_send((tx, frame_callback)) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full((tx, _)))
            | Err(std::sync::mpsc::TrySendError::Disconnected((tx, _))) => {
                // Note: support could be added for being called again after end of prior stream.
                tx.send(Err(anyhow!(
                    "Can't call ScapDefaultTargetCaptureSource::stream multiple times."
                )))
                .ok();
            }
        }
        to_dyn_screen_capture_stream(rx, foreground_executor)
    }
}

fn new_scap_capturer(target: Option<scap::Target>) -> Result<scap::capturer::Capturer> {
    scap::capturer::Capturer::build(scap::capturer::Options {
        fps: 60,
        show_cursor: true,
        show_highlight: true,
        #[cfg(target_os = "macos")]
        output_type: scap::frame::FrameType::BGRAFrame,
        // Note that the actual frame output type may differ.
        #[cfg(not(target_os = "macos"))]
        output_type: scap::frame::FrameType::YUVFrame,
        output_resolution: scap::capturer::Resolution::Captured,
        crop_area: None,
        target,
        excluded_targets: None,
    })
}

fn run_capture(
    mut capturer: scap::capturer::Capturer,
    metadata: SourceMetadata,
    frame_callback: Box<dyn Fn(ScreenCaptureFrame) + Send>,
    stream_tx: oneshot::Sender<Result<ScapStream>>,
) {
    let cancel_stream = Arc::new(AtomicBool::new(false));
    let stream_send_result = stream_tx.send(Ok(ScapStream {
        cancel_stream: cancel_stream.clone(),
        metadata,
    }));
    if stream_send_result.is_err() {
        capturer.stop_capture();
        return;
    }
    while !cancel_stream.load(std::sync::atomic::Ordering::SeqCst) {
        match next_capture_frame(&capturer) {
            Ok(Some(frame)) => frame_callback(frame),
            Ok(None) => continue,
            Err(err) => {
                log::error!("Halting screen capture due to error: {err}");
                break;
            }
        }
    }
    capturer.stop_capture();
}

struct ScapStream {
    cancel_stream: Arc<AtomicBool>,
    metadata: SourceMetadata,
}

impl ScreenCaptureStream for ScapStream {
    fn metadata(&self) -> Result<SourceMetadata> {
        Ok(self.metadata.clone())
    }
}

impl Drop for ScapStream {
    fn drop(&mut self) {
        self.cancel_stream.store(true, atomic::Ordering::SeqCst);
    }
}

#[cfg(target_os = "macos")]
fn next_capture_frame(capturer: &scap::capturer::Capturer) -> Result<Option<ScreenCaptureFrame>> {
    Ok(capturer
        .raw()
        .get_next_pixel_buffer_timeout(std::time::Duration::from_millis(100))?
        .map(|frame| ScreenCaptureFrame(frame.as_core_video())))
}

#[cfg(not(target_os = "macos"))]
fn next_capture_frame(capturer: &scap::capturer::Capturer) -> Result<Option<ScreenCaptureFrame>> {
    capturer
        .get_next_frame()
        .map(|frame| Some(ScreenCaptureFrame(frame)))
}

#[cfg(target_os = "macos")]
fn display_size(display: &scap::Display) -> Size<DevicePixels> {
    display
        .raw_handle
        .display_mode()
        .map_or_else(Size::default, |mode| {
            size(
                DevicePixels(mode.pixel_width() as i32),
                DevicePixels(mode.pixel_height() as i32),
            )
        })
}

#[cfg(not(target_os = "macos"))]
fn display_size(display: &scap::Display) -> Size<DevicePixels> {
    size(
        DevicePixels(display.width as i32),
        DevicePixels(display.height as i32),
    )
}

#[cfg(target_os = "macos")]
fn frame_size(frame: &ScreenCaptureFrame) -> Size<DevicePixels> {
    size(
        DevicePixels(frame.0.get_width() as i32),
        DevicePixels(frame.0.get_height() as i32),
    )
}

#[cfg(not(target_os = "macos"))]
fn frame_size(frame: &ScreenCaptureFrame) -> Size<DevicePixels> {
    let (width, height) = match &frame.0 {
        scap::frame::Frame::YUVFrame(frame) => (frame.width, frame.height),
        scap::frame::Frame::RGB(frame) => (frame.width, frame.height),
        scap::frame::Frame::RGBx(frame) => (frame.width, frame.height),
        scap::frame::Frame::XBGR(frame) => (frame.width, frame.height),
        scap::frame::Frame::BGRx(frame) => (frame.width, frame.height),
        scap::frame::Frame::BGR0(frame) => (frame.width, frame.height),
        scap::frame::Frame::BGRA(frame) => (frame.width, frame.height),
    };
    size(DevicePixels(width), DevicePixels(height))
}

fn metadata_for_target(target: Option<&Target>, resolution: Size<DevicePixels>) -> SourceMetadata {
    let (id, label) = match target {
        Some(Target::Display(display)) => (display.id as u64, Some(display.title.clone().into())),
        Some(Target::Window(window)) => (window.id as u64, Some(window.title.clone().into())),
        None => (0, None),
    };

    SourceMetadata {
        id,
        label,
        #[cfg(target_os = "macos")]
        is_main: match target {
            Some(Target::Display(display)) => Some(display.raw_handle.is_main()),
            _ => None,
        },
        #[cfg(not(target_os = "macos"))]
        is_main: None,
        resolution,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_target_metadata_uses_the_first_frame_resolution() {
        let resolution = size(DevicePixels(1920), DevicePixels(1080));
        let metadata = metadata_for_target(None, resolution);

        assert_eq!(metadata.id, 0);
        assert_eq!(metadata.label, None);
        assert_eq!(metadata.is_main, None);
        assert_eq!(metadata.resolution, resolution);
    }
}

/// This is used by `get_screen_targets` and `start_default_target_screen_capture` to turn their
/// results into `Rc<dyn ScreenCaptureSource>`. They need to `Send` their capture source, and so
/// the capture source structs are used as `Rc<dyn ScreenCaptureSource>` is not `Send`.
fn to_dyn_screen_capture_sources<T: ScreenCaptureSource + 'static>(
    sources_rx: oneshot::Receiver<Result<Vec<T>>>,
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Vec<Rc<dyn ScreenCaptureSource>>>> {
    let (dyn_sources_tx, dyn_sources_rx) = oneshot::channel();
    foreground_executor
        .spawn(async move {
            match sources_rx.await {
                Ok(Ok(results)) => dyn_sources_tx
                    .send(Ok(results
                        .into_iter()
                        .map(|source| Rc::new(source) as Rc<dyn ScreenCaptureSource>)
                        .collect::<Vec<_>>()))
                    .ok(),
                Ok(Err(err)) => dyn_sources_tx.send(Err(err)).ok(),
                Err(oneshot::Canceled) => None,
            }
        })
        .detach();
    dyn_sources_rx
}

/// Same motivation as `to_dyn_screen_capture_sources` above.
fn to_dyn_screen_capture_stream<T: ScreenCaptureStream + 'static>(
    sources_rx: oneshot::Receiver<Result<T>>,
    foreground_executor: &ForegroundExecutor,
) -> oneshot::Receiver<Result<Box<dyn ScreenCaptureStream>>> {
    let (dyn_sources_tx, dyn_sources_rx) = oneshot::channel();
    foreground_executor
        .spawn(async move {
            match sources_rx.await {
                Ok(Ok(stream)) => dyn_sources_tx
                    .send(Ok(Box::new(stream) as Box<dyn ScreenCaptureStream>))
                    .ok(),
                Ok(Err(err)) => dyn_sources_tx.send(Err(err)).ok(),
                Err(oneshot::Canceled) => None,
            }
        })
        .detach();
    dyn_sources_rx
}
