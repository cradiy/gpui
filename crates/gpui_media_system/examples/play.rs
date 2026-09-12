use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_media::{MediaSource, VideoPlayer, VideoPlayerEvent};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: cargo run -p gpui_media_system --example play -- <video file or URI>");
        std::process::exit(2);
    });
    let source = MediaSource::parse(&input)?;
    let title = format!("gpui_media · {}", source.display_name());

    gpui_media_system::SystemBackend::initialize()?;
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(680.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title(&title);
                #[cfg(target_os = "linux")]
                if let Some(gpu) = window.gpu_specs() {
                    eprintln!(
                        "gpui_media: renderer={} drm_device={:?} native_nv12={} modifiers={:?}",
                        gpu.device_name,
                        gpu.drm_render_device,
                        gpu.supports_native_nv12_dma_buf_import,
                        gpu.native_nv12_dma_buf_modifiers
                    );
                }
                let player = cx.new(|cx| {
                    VideoPlayer::builder(source, gpui_media_system::SystemBackend).build_in_window(window, cx)
                        .expect("failed to create video player")
                });

                #[cfg(target_os = "linux")]
                let mut reported_frame_layout = false;
                cx.subscribe(&player, move |_, event, _| match event {
                    VideoPlayerEvent::FrameTransportChanged(transport) => {
                        eprintln!("gpui_media: frame transport changed to {transport:?}");
                    }
                    #[cfg(target_os = "linux")]
                    VideoPlayerEvent::FrameReady(frame) if !reported_frame_layout => {
                        reported_frame_layout = true;
                        if let gpui::SurfaceFrameBacking::DmaBuf(dma_buf) =
                            frame.surface().backing()
                        {
                            eprintln!(
                                "gpui_media: DMA-BUF modifier={:#018x} native_image={} objects={} planes={} producer_device={:?}",
                                dma_buf.drm_modifier(),
                                dma_buf.image().is_some(),
                                dma_buf.image().map_or(0, |image| image.objects().len()),
                                dma_buf.image().map_or_else(
                                    || dma_buf.planes().len(),
                                    |image| image.planes().len()
                                ),
                                dma_buf.image().and_then(|image| image.drm_device())
                            );
                        }
                    }
                    VideoPlayerEvent::DmaBufImportFailed(reason) => {
                        eprintln!("gpui_media: DMA-BUF import failed, switching to CPU output: {reason}");
                    }
                    _ => {}
                })
                .detach();
                player
            },
        );
        if let Err(error) = result {
            eprintln!("gpui_media: failed to open window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });

    Ok(())
}
