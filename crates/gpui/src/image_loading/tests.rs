use super::*;
use crate::BackgroundExecutor;
use crate::{Asset, Image, ImageAssetLoader, Resource, TestAppContext};
use futures::{FutureExt, io::Cursor as AsyncCursor};
use image::{
    Delay, Rgba, RgbaImage,
    codecs::{gif::GifEncoder, webp::WebPEncoder},
};
use std::{
    pin::Pin,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    task::{Context, Poll},
};

fn renderer() -> SvgRenderer {
    SvgRenderer::new(Arc::new(()))
}

fn frame() -> Frame {
    Frame::from_parts(
        RgbaImage::from_pixel(4, 3, Rgba([20, 40, 60, 255])),
        0,
        0,
        Delay::from_numer_denom_ms(20, 1),
    )
}

fn gif(frame_count: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    GifEncoder::new(&mut bytes)
        .encode_frames((0..frame_count).map(|_| frame()))
        .unwrap();
    bytes
}

fn webp(frame_count: usize) -> Vec<u8> {
    fn chunk(output: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
        output.extend_from_slice(tag);
        output.extend_from_slice(&(data.len() as u32).to_le_bytes());
        output.extend_from_slice(data);
        if !data.len().is_multiple_of(2) {
            output.push(0);
        }
    }
    let mut encoded = Vec::new();
    WebPEncoder::new_lossless(&mut encoded)
        .encode(frame().buffer(), 4, 3, image::ExtendedColorType::Rgba8)
        .unwrap();
    let mut payload = b"WEBP".to_vec();
    chunk(&mut payload, b"VP8X", &[2, 0, 0, 0, 3, 0, 0, 2, 0, 0]);
    chunk(&mut payload, b"ANIM", &[0; 6]);
    for _ in 0..frame_count {
        let mut data = vec![0, 0, 0, 0, 0, 0, 3, 0, 0, 2, 0, 0, 20, 0, 0, 2];
        data.extend_from_slice(&encoded[12..]);
        chunk(&mut payload, b"ANMF", &data);
    }
    let mut bytes = b"RIFF".to_vec();
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    bytes
}

fn decode(bytes: &[u8], limits: ImageLoadLimits) -> Result<Arc<RenderImage>, ImageCacheError> {
    decode_image(bytes, image::guess_format(bytes).ok(), &renderer(), limits)
}

fn assert_limit(result: Result<Arc<RenderImage>, ImageCacheError>, expected: &str) {
    match result.unwrap_err() {
        ImageCacheError::LimitExceeded {
            resource,
            actual,
            limit,
        } => {
            assert_eq!(resource, expected);
            assert!(actual > limit);
        }
        error => panic!("expected {expected} limit, got {error}"),
    }
}

#[test]
fn static_image_limits_and_bgra_output() {
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(frame().into_buffer())
        .write_to(&mut bytes, ImageFormat::Bmp)
        .unwrap();
    let bytes = bytes.into_inner();
    let limits = ImageLoadLimits {
        max_input_bytes: bytes.len() as u64,
        max_width: 4,
        max_height: 3,
        max_frames: 1,
        max_decoded_bytes: 48,
    };
    let image = decode(&bytes, limits).unwrap();
    assert_eq!(image.as_bytes(0).unwrap(), [60, 40, 20, 255].repeat(12));
    for (limits, resource) in [
        (
            ImageLoadLimits {
                max_input_bytes: limits.max_input_bytes - 1,
                ..limits
            },
            "input bytes",
        ),
        (
            ImageLoadLimits {
                max_width: 3,
                ..limits
            },
            "width",
        ),
        (
            ImageLoadLimits {
                max_height: 2,
                ..limits
            },
            "height",
        ),
        (
            ImageLoadLimits {
                max_decoded_bytes: 47,
                ..limits
            },
            "decoded bytes",
        ),
        (
            ImageLoadLimits {
                max_frames: 0,
                ..limits
            },
            "frame count",
        ),
    ] {
        assert_limit(decode(&bytes, limits), resource);
    }
}

#[test]
fn animations_obey_frame_and_aggregate_pixel_limits() {
    for bytes in [gif(3), webp(3)] {
        let limits = ImageLoadLimits {
            max_frames: 3,
            max_decoded_bytes: 144,
            ..Default::default()
        };
        let image = decode(&bytes, limits).unwrap();
        assert_eq!(image.frame_count(), 3);
        for index in 0..3 {
            assert_eq!(image.as_bytes(index).unwrap(), [60, 40, 20, 255].repeat(12));
            assert_eq!(image.delay(index), Delay::from_numer_denom_ms(20, 1));
        }
        assert_limit(
            decode(
                &bytes,
                ImageLoadLimits {
                    max_frames: 2,
                    ..limits
                },
            ),
            "frame count",
        );
        assert_limit(
            decode(
                &bytes,
                ImageLoadLimits {
                    max_decoded_bytes: 143,
                    ..limits
                },
            ),
            "decoded bytes",
        );
    }
}

#[test]
fn in_memory_images_share_animation_limits() {
    let image = Image::from_bytes(crate::ImageFormat::Webp, webp(3));
    let limits = ImageLoadLimits {
        max_frames: 2,
        ..Default::default()
    };
    assert_limit(
        image.to_image_data_with_limits(renderer(), limits),
        "frame count",
    );
    assert_eq!(image.to_image_data(renderer()).unwrap().frame_count(), 3);
}

#[test]
fn svg_limits_include_raster_scale() {
    let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"><rect width="4" height="3" fill="red"/></svg>"#;
    let limits = ImageLoadLimits {
        max_width: 8,
        max_height: 6,
        max_decoded_bytes: 192,
        ..Default::default()
    };
    assert_eq!(
        decode(bytes, limits).unwrap().as_bytes(0).unwrap().len(),
        192
    );
    assert_limit(
        decode(
            bytes,
            ImageLoadLimits {
                max_width: 7,
                ..limits
            },
        ),
        "width",
    );
    assert_limit(
        decode(
            bytes,
            ImageLoadLimits {
                max_decoded_bytes: 191,
                ..limits
            },
        ),
        "decoded bytes",
    );
}

struct BodyReader {
    remaining: usize,
    read: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    pending: bool,
}

impl AsyncRead for BodyReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.pending {
            return Poll::Pending;
        }
        let len = self.remaining.min(buffer.len());
        buffer[..len].fill(0);
        self.remaining -= len;
        self.read.fetch_add(len, Ordering::Relaxed);
        Poll::Ready(Ok(len))
    }
}

impl Drop for BodyReader {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::Relaxed);
    }
}

#[crate::test]
async fn http_load_stops_at_the_input_limit(cx: &mut TestAppContext) {
    let read = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let client = http_client::FakeHttpClient::create({
        let read = read.clone();
        let dropped = dropped.clone();
        move |_| {
            let reader = BodyReader {
                remaining: 1_000_000,
                read: read.clone(),
                dropped: dropped.clone(),
                pending: false,
            };
            async move {
                Ok(http_client::Response::builder()
                    .status(200)
                    .header("content-length", "1")
                    .body(http_client::AsyncBody::from_reader(reader))
                    .unwrap())
            }
        }
    });
    let future = cx.update(|cx| {
        cx.set_http_client(client);
        cx.set_global(ImageLoadLimits {
            max_input_bytes: 100,
            ..Default::default()
        });
        let future = ImageAssetLoader::load(Resource::Uri("https://example.test/image".into()), cx);
        cx.background_executor().spawn(future)
    });
    assert_limit(future.await, "input bytes");
    assert_eq!(read.load(Ordering::Relaxed), 101);
    assert!(dropped.load(Ordering::Relaxed));
}

#[crate::test]
async fn file_load_checks_input_size_before_decoding(cx: &mut TestAppContext) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/image/black-cat-typing.gif");
    let task = cx.update(|cx| {
        cx.set_global(ImageLoadLimits {
            max_input_bytes: 1,
            ..Default::default()
        });
        let future = ImageAssetLoader::load(Resource::Path(path.into()), cx);
        cx.background_executor().spawn(future)
    });
    assert_limit(task.await, "input bytes");
}

#[test]
fn input_limit_accepts_exact_length_and_releases_cancelled_reader() {
    futures::executor::block_on(async {
        let limits = ImageLoadLimits {
            max_input_bytes: 4,
            ..Default::default()
        };
        assert_eq!(
            read_image_bytes(AsyncCursor::new(vec![1; 4]), limits)
                .await
                .unwrap(),
            vec![1; 4]
        );
        let dropped = Arc::new(AtomicBool::new(false));
        let reader = BodyReader {
            remaining: 8,
            read: Arc::new(AtomicUsize::new(0)),
            dropped: dropped.clone(),
            pending: true,
        };
        let mut future = Box::pin(read_image_bytes(reader, limits));
        assert!(future.as_mut().now_or_never().is_none());
        drop(future);
        assert!(dropped.load(Ordering::Relaxed));
    });
}

#[crate::test]
async fn incremental_frames_match_eager_decode_and_release_evicted_frames(
    executor: BackgroundExecutor,
) {
    for bytes in [gif(3), webp(3)] {
        let expected = decode(&bytes, ImageLoadLimits::default()).unwrap();
        let poster = load_image(
            &bytes,
            image::guess_format(&bytes).ok(),
            &renderer(),
            ImageLoadLimits {
                max_decoded_bytes: 96,
                ..Default::default()
            },
            ImageAnimationOptions { cache_frames: 2 },
        )
        .unwrap();
        assert_eq!(poster.frame_count(), 1);
        let source = poster.animation().unwrap();
        assert_eq!(source.frame_count(), 3);
        let first = source.frame(1, &executor).await.unwrap();
        let first_id = first.id;
        let weak = Arc::downgrade(&first);
        let second = source.frame(2, &executor).await.unwrap();
        assert_eq!(first.as_bytes(0), expected.as_bytes(1));
        assert_eq!(second.as_bytes(0), expected.as_bytes(2));
        assert!(weak.upgrade().is_some());
        drop(first);
        assert!(weak.upgrade().is_none());
        let first = source.frame(1, &executor).await.unwrap();
        assert_eq!(first.id, first_id);
        assert_eq!(first.as_bytes(0), expected.as_bytes(1));
        for index in [0, 1, 2, 0, 1, 2] {
            let actual = source.frame(index, &executor).await.unwrap();
            assert_eq!(actual.as_bytes(0), expected.as_bytes(index));
            assert_eq!(actual.delay(0), expected.delay(index));
        }
        assert!(source.frame(3, &executor).await.is_err());
    }
}

fn composited_gif() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut bytes, 4, 3, &[]).unwrap();
        for (left, top, width, height, color, dispose) in [
            (0, 0, 4, 3, [80, 40, 20, 255], gif::DisposalMethod::Keep),
            (1, 1, 2, 1, [20, 80, 40, 255], gif::DisposalMethod::Previous),
            (
                0,
                0,
                1,
                2,
                [40, 20, 80, 255],
                gif::DisposalMethod::Background,
            ),
            (0, 0, 4, 3, [0, 0, 0, 0], gif::DisposalMethod::Keep),
        ] {
            let mut pixels = color.repeat(width as usize * height as usize);
            let mut frame = gif::Frame::from_rgba_speed(width, height, &mut pixels, 10);
            frame.left = left;
            frame.top = top;
            frame.dispose = dispose;
            frame.delay = 3;
            encoder.write_frame(&frame).unwrap();
        }
    }
    bytes
}

#[crate::test]
async fn incremental_gif_preserves_subframes_transparency_and_disposal(
    executor: BackgroundExecutor,
) {
    let bytes = composited_gif();
    let expected = decode(&bytes, ImageLoadLimits::default()).unwrap();
    let poster = load_image(
        &bytes,
        Some(ImageFormat::Gif),
        &renderer(),
        ImageLoadLimits::default(),
        ImageAnimationOptions { cache_frames: 2 },
    )
    .unwrap();
    let source = poster.animation().unwrap();
    for index in [0, 1, 2, 3, 0, 1, 3, 2] {
        let actual = source.frame(index, &executor).await.unwrap();
        assert_eq!(
            actual.as_bytes(0),
            expected.as_bytes(index),
            "frame {index}"
        );
        assert_eq!(actual.delay(0), expected.delay(index));
    }
}

#[crate::test]
fn animation_playback_keeps_current_frame_until_ready(cx: &mut TestAppContext) {
    let bytes = gif(3);
    let poster = load_image(
        &bytes,
        Some(ImageFormat::Gif),
        &renderer(),
        ImageLoadLimits::default(),
        ImageAnimationOptions::default(),
    )
    .unwrap();
    let mut player = AnimationPlayback::new(poster.clone(), poster.animation().unwrap().clone());
    let now = scheduler::Instant::now();
    let executor = cx.background_executor.clone();
    let first = player.update(now, true, &executor).unwrap();
    let later = now + std::time::Duration::from_secs(1);
    assert_eq!(player.update(later, true, &executor).unwrap().id, first.id);
    cx.run_until_parked();
    let second = player.update(later, true, &executor).unwrap();
    assert_ne!(second.id, first.id);
    cx.run_until_parked();
    let paused = later + std::time::Duration::from_secs(1);
    assert_eq!(
        player.update(paused, false, &executor).unwrap().id,
        second.id
    );
    assert_eq!(
        player.update(paused, true, &executor).unwrap().id,
        second.id
    );
    assert_ne!(
        player
            .update(
                paused + std::time::Duration::from_millis(20),
                true,
                &executor
            )
            .unwrap()
            .id,
        second.id
    );
}

#[test]
fn incremental_loading_checks_metadata_and_cache_budgets() {
    for bytes in [gif(3), webp(3)] {
        assert_limit(
            load_image(
                &bytes,
                image::guess_format(&bytes).ok(),
                &renderer(),
                ImageLoadLimits {
                    max_frames: 2,
                    ..Default::default()
                },
                ImageAnimationOptions::default(),
            ),
            "frame count",
        );
        assert_limit(
            load_image(
                &bytes,
                image::guess_format(&bytes).ok(),
                &renderer(),
                ImageLoadLimits {
                    max_decoded_bytes: 95,
                    ..Default::default()
                },
                ImageAnimationOptions::default(),
            ),
            "decoded bytes",
        );
    }
}

#[crate::test(iterations = 10)]
async fn concurrent_animation_requests_and_cancellation_preserve_frames(
    executor: BackgroundExecutor,
) {
    let bytes = composited_gif();
    let expected = decode(&bytes, ImageLoadLimits::default()).unwrap();
    let poster = load_image(
        &bytes,
        Some(ImageFormat::Gif),
        &renderer(),
        ImageLoadLimits::default(),
        ImageAnimationOptions { cache_frames: 2 },
    )
    .unwrap();
    let source = poster.animation().unwrap();
    let cancelled = source.frame(3, &executor);
    let first = source.frame(1, &executor);
    let third = source.frame(3, &executor);
    drop(cancelled);
    let (first, third) = futures::join!(first, third);
    assert_eq!(first.unwrap().as_bytes(0), expected.as_bytes(1));
    assert_eq!(third.unwrap().as_bytes(0), expected.as_bytes(3));
}

#[crate::test]
async fn image_asset_loader_returns_an_incremental_animation(cx: &mut TestAppContext) {
    let bytes = gif(3);
    let client = http_client::FakeHttpClient::create(move |_| {
        let bytes = bytes.clone();
        async move {
            Ok(http_client::Response::builder()
                .status(200)
                .body(bytes.into())
                .unwrap())
        }
    });
    let task = cx.update(|cx| {
        cx.set_http_client(client);
        cx.set_global(ImageLoadLimits {
            max_decoded_bytes: 96,
            ..Default::default()
        });
        let future = ImageAssetLoader::load(
            Resource::Uri("https://example.test/animation.gif".into()),
            cx,
        );
        cx.background_executor().spawn(future)
    });
    let poster = task.await.unwrap();
    assert_eq!(poster.frame_count(), 1);
    assert_eq!(poster.animation().unwrap().frame_count(), 3);
    let weak = Arc::downgrade(poster.animation().unwrap());
    let pending = poster
        .animation()
        .unwrap()
        .frame(2, &cx.background_executor);
    drop(pending);
    drop(poster);
    cx.run_until_parked();
    assert!(weak.upgrade().is_none());
}
