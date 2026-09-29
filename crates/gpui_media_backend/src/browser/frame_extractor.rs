use super::*;
use std::{future::Future, pin::Pin};

#[wasm_bindgen::prelude::wasm_bindgen(module = "/src/browser/frame_extractor.js")]
extern "C" {
    #[wasm_bindgen::prelude::wasm_bindgen(catch, js_name = extractFrame)]
    fn extract_frame(
        video: &HtmlVideoElement,
        seconds: f64,
        timeout: f64,
    ) -> Result<js_sys::Promise, wasm_bindgen::JsValue>;
}
struct VideoOwner(HtmlVideoElement);
impl Drop for VideoOwner {
    fn drop(&mut self) {
        let _ = self.0.pause();
        let _ = self.0.remove_attribute("src");
        self.0.load();
    }
}
struct Extractor {
    video: BrowserResource<VideoOwner>,
    timeout: Duration,
    handle: FrameHandle,
    sequence: u64,
}
pub(super) fn open(
    request: FrameExtractorBackendRequest,
) -> MediaResult<Box<dyn FrameExtractionSession>> {
    initialize()?;
    if request.video_decoder != VideoDecoderPolicy::Auto {
        return Err(MediaError::unsupported(
            "browsers choose the video decoder; explicit decoder policies are unavailable",
        ));
    }
    if request.timeout.as_millis() == 0 || request.timeout.as_millis() > i32::MAX as u128 {
        return Err(MediaError::invalid_input(
            "browser extraction timeout must be between 1 ms and 2147483647 ms",
        ));
    }
    if !["http:", "https:", "blob:", "data:"]
        .iter()
        .any(|scheme| request.source.uri().starts_with(scheme))
        || request.source.network_options() != &NetworkSourceOptions::default()
    {
        return Err(MediaError::unsupported(
            "browser extraction requires a browser-accessible URL and browser-managed networking",
        ));
    }
    let video: HtmlVideoElement = web_sys::window()
        .unwrap()
        .document()
        .ok_or_else(|| MediaError::unsupported("no browser document"))?
        .create_element("video")
        .map_err(js_error)?
        .dyn_into()
        .map_err(js_error)?;
    video.set_cross_origin(Some("anonymous"));
    video.set_muted(true);
    video.set_preload("auto");
    video.set_attribute("playsinline", "").map_err(js_error)?;
    video.set_src(request.source.uri());
    video.load();
    Ok(Box::new(Extractor {
        video: BrowserResource::new(VideoOwner(video)),
        timeout: request.timeout,
        handle: FrameHandle::new(),
        sequence: 0,
    }))
}
impl FrameExtractionSession for Extractor {
    fn initial_frame(&mut self) -> MediaResult<Arc<VideoFrame>> {
        Err(MediaError::unsupported(
            "browser frame extraction requires the asynchronous API",
        ))
    }
    fn frame_at(&mut self, _: Duration, _: SeekMode) -> MediaResult<Arc<VideoFrame>> {
        self.initial_frame()
    }
    fn frame_at_async(
        &mut self,
        position: Duration,
        _mode: SeekMode,
    ) -> Pin<Box<dyn Future<Output = MediaResult<Arc<VideoFrame>>>>> {
        let owner = self.video.clone();
        let promise = owner
            .with(|owner| {
                extract_frame(
                    &owner.0,
                    position.as_secs_f64(),
                    self.timeout.as_secs_f64() * 1000.,
                )
                .map_err(js_error)
            })
            .unwrap_or_else(|| {
                Err(MediaError::unsupported(
                    "browser extraction must run on its owner thread",
                ))
            });
        self.sequence = self.sequence.wrapping_add(1);
        let sequence = self.sequence;
        let handle = self.handle;
        Box::pin(async move {
            let frame = wasm_bindgen_futures::JsFuture::from(promise?)
                .await
                .map_err(|error| {
                    if js_sys::Reflect::get(&error, &"name".into())
                        .ok()
                        .and_then(|v| v.as_string())
                        .as_deref()
                        == Some("TimeoutError")
                    {
                        MediaError::timeout("browser frame extraction timed out")
                    } else {
                        js_error(error)
                    }
                })?;
            let frame: web_sys::VideoFrame = frame.dyn_into().map_err(js_error)?;
            let timestamp = Duration::from_secs_f64((frame.timestamp() / 1_000_000.).max(0.));
            let snapshot = BrowserVideoFrame::new(&frame).map_err(js_error);
            frame.close();
            let snapshot = snapshot?;
            let size = FrameSize::new(snapshot.width() as i32, snapshot.height() as i32);
            let buffer = FrameBuffer::with_backing(
                handle,
                sequence,
                size,
                FrameRect {
                    origin: Default::default(),
                    size,
                },
                size,
                PixelFormat::Rgba8,
                FrameBacking::Browser(snapshot),
                Default::default(),
            )?;
            drop(owner);
            Ok(Arc::new(VideoFrame::new(
                Arc::new(buffer),
                Some(timestamp),
                None,
            )))
        })
    }
}
