use crate::{Global, ImageCacheError, RenderImage, SvgRenderer};
use futures::{AsyncRead, AsyncReadExt};
use image::{
    AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageFormat, ImageReader,
    codecs::{gif::GifDecoder, webp::WebPDecoder},
};
use smallvec::SmallVec;
use std::{io::Cursor, sync::Arc};

mod animation;
mod playback;
pub use animation::{ImageAnimation, ImageAnimationOptions};
pub(crate) use playback::AnimationPlayback;

pub(crate) fn load_image(
    bytes: &[u8],
    format: Option<ImageFormat>,
    svg_renderer: &SvgRenderer,
    limits: ImageLoadLimits,
    options: ImageAnimationOptions,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    if matches!(format, Some(ImageFormat::Gif | ImageFormat::WebP)) {
        limits.check_input(bytes.len() as u64)?;
        animation::load(Arc::from(bytes), format.unwrap(), limits, options)
    } else {
        decode_image(bytes, format, svg_renderer, limits)
    }
}

/// Per-image resource limits for GPUI's built-in image loaders.
///
/// Install with `cx.set_global(ImageLoadLimits { ..Default::default() })` before
/// loading images. Each new load captures the current limits; cached images are
/// unchanged. Custom loaders and already decoded `RenderImage` values are excluded.
/// Decoder scratch allocations and SVG parsing are not strictly memory-bounded.
/// GIF and animated WebP loaders retain a poster and a bounded decoded frame cache;
/// frames held by callers can outlive that cache.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageLoadLimits {
    /// Maximum encoded input size, in bytes. Defaults to 64 MiB.
    pub max_input_bytes: u64,
    /// Maximum decoded frame width. Defaults to 16,384 pixels.
    pub max_width: u32,
    /// Maximum decoded frame height. Defaults to 16,384 pixels.
    pub max_height: u32,
    /// Maximum total BGRA pixel bytes retained for an image, across all frames.
    /// Defaults to 256 MiB. Also used as a best-effort decoder allocation limit.
    pub max_decoded_bytes: u64,
    /// Maximum frame count. Defaults to 512.
    pub max_frames: u32,
}

impl Default for ImageLoadLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 64 * 1024 * 1024,
            max_width: 16_384,
            max_height: 16_384,
            max_decoded_bytes: 256 * 1024 * 1024,
            max_frames: 512,
        }
    }
}

impl Global for ImageLoadLimits {}

impl ImageLoadLimits {
    pub(crate) fn check(
        resource: &'static str,
        actual: u64,
        limit: u64,
    ) -> Result<(), ImageCacheError> {
        if actual > limit {
            return Err(ImageCacheError::LimitExceeded {
                resource,
                actual,
                limit,
            });
        }
        Ok(())
    }

    pub(crate) fn check_input(&self, bytes: u64) -> Result<(), ImageCacheError> {
        Self::check("input bytes", bytes, self.max_input_bytes)
    }

    pub(crate) fn check_frame(&self, width: u32, height: u32) -> Result<u64, ImageCacheError> {
        Self::check("width", width.into(), self.max_width.into())?;
        Self::check("height", height.into(), self.max_height.into())?;
        Self::check("frame count", 1, self.max_frames.into())?;
        let bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("decoded image byte size overflows"))?;
        Self::check("decoded bytes", bytes, self.max_decoded_bytes)?;
        Ok(bytes)
    }

    fn decoder_limits(&self) -> image::Limits {
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(self.max_width);
        limits.max_image_height = Some(self.max_height);
        limits.max_alloc = Some(self.max_decoded_bytes);
        limits
    }

    fn prepare_decoder(&self, decoder: &mut impl ImageDecoder) -> Result<(), ImageCacheError> {
        let (width, height) = decoder.dimensions();
        self.check_frame(width, height)?;
        Self::check(
            "decoded bytes",
            decoder.total_bytes(),
            self.max_decoded_bytes,
        )?;
        decoder.set_limits(self.decoder_limits())?;
        Ok(())
    }
}

pub(crate) async fn read_image_bytes(
    mut reader: impl AsyncRead + Unpin,
    limits: ImageLoadLimits,
) -> Result<Vec<u8>, ImageCacheError> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let remaining = limits.max_input_bytes.saturating_sub(bytes.len() as u64);
        let read_size = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let count = reader.read(&mut buffer[..read_size]).await?;
        if count == 0 {
            return Ok(bytes);
        }
        limits.check_input((bytes.len() as u64).saturating_add(count as u64))?;
        bytes.extend_from_slice(&buffer[..count]);
    }
}

pub(crate) fn decode_image(
    bytes: &[u8],
    format: Option<ImageFormat>,
    svg_renderer: &SvgRenderer,
    limits: ImageLoadLimits,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    limits.check_input(bytes.len() as u64)?;
    let Some(format) = format else {
        return svg_renderer.render_single_frame_with_limits(bytes, 1.0, limits);
    };

    let frames = match format {
        ImageFormat::Gif => {
            let mut decoder = GifDecoder::new(Cursor::new(bytes))?;
            limits.prepare_decoder(&mut decoder)?;
            collect_frames(decoder.into_frames(), limits)?
        }
        ImageFormat::WebP => {
            let mut decoder = WebPDecoder::new(Cursor::new(bytes))?;
            limits.prepare_decoder(&mut decoder)?;
            if decoder.has_animation() {
                let _ = decoder.set_background_color(image::Rgba([0, 0, 0, 0]));
                collect_frames(decoder.into_frames(), limits)?
            } else {
                single_frame(decoder)?
            }
        }
        _ => {
            let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
            let mut header_limits = image::Limits::default();
            header_limits.max_alloc = Some(limits.max_decoded_bytes);
            reader.limits(header_limits);
            let mut decoder = reader.into_decoder()?;
            limits.prepare_decoder(&mut decoder)?;
            single_frame(decoder)?
        }
    };
    Ok(Arc::new(RenderImage::new(frames)))
}

fn single_frame(decoder: impl ImageDecoder) -> Result<SmallVec<[Frame; 1]>, ImageCacheError> {
    let mut frame = Frame::new(DynamicImage::from_decoder(decoder)?.into_rgba8());
    convert_to_bgra(&mut frame);
    Ok(SmallVec::from_elem(frame, 1))
}

fn collect_frames(
    frames: image::Frames<'_>,
    limits: ImageLoadLimits,
) -> Result<SmallVec<[Frame; 1]>, ImageCacheError> {
    let mut result = SmallVec::new();
    let mut decoded_bytes = 0u64;
    for frame in frames {
        let mut frame = frame?;
        ImageLoadLimits::check(
            "frame count",
            result.len() as u64 + 1,
            limits.max_frames.into(),
        )?;
        let (width, height) = frame.buffer().dimensions();
        decoded_bytes = decoded_bytes.saturating_add(limits.check_frame(width, height)?);
        ImageLoadLimits::check("decoded bytes", decoded_bytes, limits.max_decoded_bytes)?;
        convert_to_bgra(&mut frame);
        result.push(frame);
    }
    if result.is_empty() {
        return Err(anyhow::anyhow!("image contains no decodable frames").into());
    }
    Ok(result)
}

fn convert_to_bgra(frame: &mut Frame) {
    for pixel in frame.buffer_mut().chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

#[cfg(test)]
mod tests;
