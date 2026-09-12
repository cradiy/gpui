use super::{ImageLoadLimits, convert_to_bgra};
use crate::{BackgroundExecutor, Global, ImageCacheError, ImageId, RenderImage, Task};
use futures::lock::Mutex;
use image::{Delay, Frame, ImageFormat, RgbaImage};
use std::{collections::VecDeque, io::Cursor, sync::Arc};

/// Cache settings for animations loaded by GPUI's built-in image loaders.
/// Install as an application global before loading images.
#[derive(Clone, Copy, Debug)]
pub struct ImageAnimationOptions {
    /// Maximum cached frames, including the poster. Must be at least two for
    /// multi-frame images. Defaults to three. The decoded byte limit can reduce
    /// this capacity.
    pub cache_frames: usize,
}

impl Default for ImageAnimationOptions {
    fn default() -> Self {
        Self { cache_frames: 3 }
    }
}
impl Global for ImageAnimationOptions {}

/// An indexed animation decoded on a background executor.
///
/// Cached frames are immutable snapshots. Holding returned frames keeps them
/// alive beyond cache eviction. Seeking behind the decoder restarts decoding
/// from the beginning; sequential playback reuses the decoder state.
pub struct ImageAnimation {
    bytes: Arc<[u8]>,
    format: ImageFormat,
    limits: ImageLoadLimits,
    frame_count: usize,
    capacity: usize,
    ids: Vec<ImageId>,
    state: Mutex<AnimationState>,
}

struct AnimationState {
    decoder: Decoder,
    next_index: usize,
    cache: VecDeque<(usize, Arc<RenderImage>)>,
}

impl ImageAnimation {
    pub(crate) fn frame_ids(&self) -> &[ImageId] {
        &self.ids
    }
    /// Number of frames in one complete animation cycle.
    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    /// Decodes or retrieves an immutable single-frame image without doing work
    /// on the calling thread. Dropping the task cancels pending work; an active
    /// codec call finishes before cancellation is observed.
    pub fn frame(
        self: &Arc<Self>,
        index: usize,
        executor: &BackgroundExecutor,
    ) -> Task<Result<Arc<RenderImage>, ImageCacheError>> {
        let source = self.clone();
        executor.spawn(async move { source.decode_frame(index).await })
    }

    async fn decode_frame(&self, index: usize) -> Result<Arc<RenderImage>, ImageCacheError> {
        if index >= self.frame_count {
            return Err(anyhow::anyhow!("animation frame index out of range").into());
        }
        let mut state = self.state.lock().await;
        if let Some((_, image)) = state.cache.iter().find(|(cached, _)| *cached == index) {
            return Ok(image.clone());
        }
        if index < state.next_index {
            state.decoder = Decoder::new(self.bytes.clone(), self.format, self.limits)?;
            state.next_index = 0;
        }
        while state.next_index <= index {
            let mut frame = match state.decoder.next() {
                Ok(frame) => frame,
                Err(error) => {
                    state.decoder = Decoder::new(self.bytes.clone(), self.format, self.limits)?;
                    state.next_index = 0;
                    return Err(error);
                }
            };
            let current = state.next_index;
            state.next_index += 1;
            if current != index {
                let mut yielded = false;
                futures::future::poll_fn(|cx| {
                    if yielded {
                        std::task::Poll::Ready(())
                    } else {
                        yielded = true;
                        cx.waker().wake_by_ref();
                        std::task::Poll::Pending
                    }
                })
                .await;
                continue;
            }
            convert_to_bgra(&mut frame);
            let mut image = RenderImage::new(vec![frame]);
            image.id = self.ids[current];
            let image = Arc::new(image);
            while state.cache.len() >= self.capacity {
                // The poster stays resident for non-animated consumers.
                state.cache.remove(1);
            }
            state.cache.push_back((current, image.clone()));
            return Ok(image);
        }
        unreachable!()
    }
}

pub(super) fn load(
    bytes: Arc<[u8]>,
    format: ImageFormat,
    limits: ImageLoadLimits,
    options: ImageAnimationOptions,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let (count, frame_bytes) = match format {
        ImageFormat::Gif => {
            let mut options = gif_options(limits);
            options.skip_frame_decoding(true);
            let mut decoder = options
                .read_info(Cursor::new(bytes.clone()))
                .map_err(codec_error)?;
            let frame_bytes =
                limits.check_frame(decoder.width().into(), decoder.height().into())?;
            let mut count = 0;
            while decoder.next_frame_info().map_err(codec_error)?.is_some() {
                count += 1;
                ImageLoadLimits::check("frame count", count as u64, limits.max_frames.into())?;
            }
            (count, frame_bytes)
        }
        ImageFormat::WebP => {
            let decoder =
                image_webp::WebPDecoder::new(Cursor::new(bytes.clone())).map_err(codec_error)?;
            let (width, height) = decoder.dimensions();
            let frame_bytes = limits.check_frame(width, height)?;
            (decoder.num_frames().max(1) as usize, frame_bytes)
        }
        _ => unreachable!(),
    };
    ImageLoadLimits::check("frame count", count as u64, limits.max_frames.into())?;
    if count == 0 {
        return Err(anyhow::anyhow!("image contains no frames").into());
    }
    if frame_bytes == 0 {
        return Err(anyhow::anyhow!("animation dimensions must be nonzero").into());
    }
    if count > 1 {
        ImageLoadLimits::check(
            "decoded bytes",
            frame_bytes.saturating_mul(2),
            limits.max_decoded_bytes,
        )?;
        if options.cache_frames < 2 {
            return Err(anyhow::anyhow!("animation cache requires at least two frames").into());
        }
    }
    let mut decoder = Decoder::new(bytes.clone(), format, limits)?;
    let mut first = decoder.next()?;
    convert_to_bgra(&mut first);
    let poster = Arc::new(RenderImage::new(vec![first]));
    if count == 1 {
        return Ok(poster);
    }
    let capacity = options
        .cache_frames
        .min(usize::try_from(limits.max_decoded_bytes / frame_bytes).unwrap_or(usize::MAX))
        .min(count);
    let mut ids = (0..count).map(|_| ImageId::allocate()).collect::<Vec<_>>();
    ids[0] = poster.id;
    let source = Arc::new(ImageAnimation {
        bytes,
        format,
        limits,
        frame_count: count,
        capacity,
        ids,
        state: Mutex::new(AnimationState {
            decoder,
            next_index: 1,
            cache: VecDeque::from([(0, poster.clone())]),
        }),
    });
    Ok(Arc::new(poster.with_animation(source)))
}

fn codec_error(error: impl std::error::Error + Send + Sync + 'static) -> ImageCacheError {
    anyhow::Error::new(error).into()
}

fn gif_options(limits: ImageLoadLimits) -> gif::DecodeOptions {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    options.check_frame_consistency(true);
    if let Some(bytes) = std::num::NonZeroU64::new(limits.max_decoded_bytes) {
        options.set_memory_limit(gif::MemoryLimit::Bytes(bytes));
    }
    options
}

enum Decoder {
    Gif {
        decoder: gif::Decoder<Cursor<Arc<[u8]>>>,
        canvas: RgbaImage,
        previous: Option<RgbaImage>,
        disposal: gif::DisposalMethod,
        rect: (u32, u32, u32, u32),
    },
    WebP(image_webp::WebPDecoder<Cursor<Arc<[u8]>>>),
}

impl Decoder {
    fn new(
        bytes: Arc<[u8]>,
        format: ImageFormat,
        limits: ImageLoadLimits,
    ) -> Result<Self, ImageCacheError> {
        match format {
            ImageFormat::Gif => {
                let decoder = gif_options(limits)
                    .read_info(Cursor::new(bytes))
                    .map_err(codec_error)?;
                let (width, height) = (decoder.width().into(), decoder.height().into());
                limits.check_frame(width, height)?;
                Ok(Self::Gif {
                    decoder,
                    canvas: RgbaImage::new(width, height),
                    previous: None,
                    disposal: gif::DisposalMethod::Any,
                    rect: (0, 0, 0, 0),
                })
            }
            ImageFormat::WebP => {
                let mut decoder =
                    image_webp::WebPDecoder::new(Cursor::new(bytes)).map_err(codec_error)?;
                let (width, height) = decoder.dimensions();
                limits.check_frame(width, height)?;
                decoder.set_memory_limit(limits.max_decoded_bytes.min(usize::MAX as u64) as usize);
                if decoder.is_animated() {
                    decoder.set_background_color([0; 4]).map_err(codec_error)?;
                }
                Ok(Self::WebP(decoder))
            }
            _ => unreachable!(),
        }
    }

    fn next(&mut self) -> Result<Frame, ImageCacheError> {
        match self {
            Self::Gif {
                decoder,
                canvas,
                previous,
                disposal,
                rect,
            } => {
                match disposal {
                    gif::DisposalMethod::Background => {
                        for y in rect.1..rect.1 + rect.3 {
                            for x in rect.0..rect.0 + rect.2 {
                                canvas.put_pixel(x, y, image::Rgba([0; 4]));
                            }
                        }
                    }
                    gif::DisposalMethod::Previous => {
                        if let Some(saved) = previous.take() {
                            *canvas = saved;
                        }
                    }
                    _ => {}
                }
                let frame = decoder
                    .read_next_frame()
                    .map_err(codec_error)?
                    .ok_or_else(|| anyhow::anyhow!("animation ended before its indexed frame"))?;
                if frame.dispose == gif::DisposalMethod::Previous {
                    *previous = Some(canvas.clone());
                }
                for y in 0..u32::from(frame.height) {
                    for x in 0..u32::from(frame.width) {
                        let offset = (y as usize * frame.width as usize + x as usize) * 4;
                        let pixel: [u8; 4] = frame.buffer[offset..offset + 4].try_into().unwrap();
                        if pixel[3] != 0 {
                            canvas.put_pixel(
                                x + u32::from(frame.left),
                                y + u32::from(frame.top),
                                image::Rgba(pixel),
                            );
                        }
                    }
                }
                *disposal = frame.dispose;
                *rect = (
                    frame.left.into(),
                    frame.top.into(),
                    frame.width.into(),
                    frame.height.into(),
                );
                Ok(Frame::from_parts(
                    canvas.clone(),
                    0,
                    0,
                    Delay::from_numer_denom_ms(u32::from(frame.delay) * 10, 1),
                ))
            }
            Self::WebP(decoder) => {
                let (width, height) = decoder.dimensions();
                let channels = if decoder.has_alpha() { 4 } else { 3 };
                let mut bytes = vec![0; width as usize * height as usize * channels];
                let delay = if decoder.is_animated() {
                    decoder.read_frame(&mut bytes).map_err(codec_error)?
                } else {
                    decoder.read_image(&mut bytes).map_err(codec_error)?;
                    0
                };
                let buffer = if channels == 4 {
                    RgbaImage::from_raw(width, height, bytes).unwrap()
                } else {
                    image::DynamicImage::ImageRgb8(
                        image::RgbImage::from_raw(width, height, bytes).unwrap(),
                    )
                    .into_rgba8()
                };
                Ok(Frame::from_parts(
                    buffer,
                    0,
                    0,
                    Delay::from_numer_denom_ms(delay, 1),
                ))
            }
        }
    }
}
