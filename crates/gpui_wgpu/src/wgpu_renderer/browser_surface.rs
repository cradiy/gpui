use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(extends = js_sys::Object)]
    type ExternalImageQueue;

    // Keep synchronous browser errors inside Rust; wgpu's wrapper unwraps them.
    #[wasm_bindgen(method, catch, structural, js_name = copyExternalImageToTexture)]
    fn copy_image(
        this: &ExternalImageQueue,
        source: &js_sys::Object,
        destination: &js_sys::Object,
        size: &js_sys::Array,
    ) -> Result<(), JsValue>;
}

#[derive(Default)]
pub(super) struct BrowserSurfaceUploader {
    use_canvas: bool,
    canvas: Option<(
        web_sys::OffscreenCanvas,
        web_sys::OffscreenCanvasRenderingContext2d,
    )>,
    reported_error: bool,
}

impl BrowserSurfaceUploader {
    pub(super) fn upload(
        &mut self,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        frame: &web_sys::VideoFrame,
    ) {
        match self.try_upload(queue, texture, frame) {
            Ok(()) => self.reported_error = false,
            Err(error) => {
                if !self.reported_error {
                    log::error!("failed to upload browser video frame: {error:?}");
                }
                self.reported_error = true;
            }
        }
    }

    fn try_upload(
        &mut self,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        frame: &web_sys::VideoFrame,
    ) -> Result<(), JsValue> {
        let queue = queue
            .as_webgpu()
            .ok_or_else(|| JsValue::from_str("Video frames require a WebGPU queue"))?;
        let destination = js_sys::Object::new();
        js_sys::Reflect::set(
            &destination,
            &"texture".into(),
            texture
                .as_webgpu()
                .ok_or_else(|| JsValue::from_str("Video frames require a WebGPU texture"))?
                .as_ref(),
        )?;
        js_sys::Reflect::set(&destination, &"premultipliedAlpha".into(), &true.into())?;
        let size = js_sys::Array::new();
        size.push(&frame.display_width().into());
        size.push(&frame.display_height().into());
        size.push(&1.into());
        let source = js_sys::Object::new();
        let queue: &ExternalImageQueue = queue.unchecked_ref();

        if !self.use_canvas {
            js_sys::Reflect::set(&source, &"source".into(), frame.as_ref())?;
            match queue.copy_image(&source, &destination, &size) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    log::debug!("Direct VideoFrame upload unavailable; using canvas: {error:?}");
                    self.use_canvas = true;
                }
            }
        }

        // Reuse a canvas per cached surface, without CPU pixel readback.
        if self.canvas.is_none() {
            let canvas =
                web_sys::OffscreenCanvas::new(frame.display_width(), frame.display_height())?;
            let context = canvas
                .get_context("2d")?
                .ok_or_else(|| JsValue::from_str("No offscreen 2D context"))?
                .dyn_into::<web_sys::OffscreenCanvasRenderingContext2d>()?;
            self.canvas = Some((canvas, context));
        }
        let (canvas, context) = self.canvas.as_ref().unwrap();
        if canvas.width() != frame.display_width() || canvas.height() != frame.display_height() {
            canvas.set_width(frame.display_width());
            canvas.set_height(frame.display_height());
        }
        context.clear_rect(0., 0., canvas.width() as f64, canvas.height() as f64);
        context.draw_image_with_video_frame(frame, 0., 0.)?;
        js_sys::Reflect::set(&source, &"source".into(), canvas.as_ref())?;
        queue.copy_image(&source, &destination, &size)
    }
}
