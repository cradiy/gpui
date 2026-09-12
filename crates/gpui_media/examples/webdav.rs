use std::{env, time::Duration};

use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_media::{
    MediaSource, NetworkSourceOptions, PlaybackState, VideoPlayer, VideoPlayerEvent,
    VideoPlayerOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = env::args().nth(1).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: GPUI_MEDIA_WEBDAV_USERNAME=user \
             GPUI_MEDIA_WEBDAV_PASSWORD=password \
             cargo run -p gpui_media --example webdav -- <direct WebDAV file URL>",
        )
    })?;
    let username = env::var("GPUI_MEDIA_WEBDAV_USERNAME").ok();
    let password = env::var("GPUI_MEDIA_WEBDAV_PASSWORD").ok();
    if username.is_some() != password.is_some() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "GPUI_MEDIA_WEBDAV_USERNAME and GPUI_MEDIA_WEBDAV_PASSWORD must be set together",
        )
        .into());
    }

    let mut network = NetworkSourceOptions::default()
        .with_user_agent("gpui-media-webdav-example/0.1")
        .with_timeout(Duration::from_secs(15))
        .with_retry_count(3)
        .with_retry_backoff(Duration::from_millis(250), Duration::from_secs(3))
        .with_buffer_duration(Duration::from_secs(5));
    if let (Some(username), Some(password)) = (username, password) {
        network = network.with_basic_auth(username, password);
    }

    let source = MediaSource::from_uri(url)?.with_network_options(network);
    let title = format!("gpui_media WebDAV · {}", source.display_name());

    gpui_media_backend::SystemBackend::initialize()?;
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(680.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title(&title);
                let player = cx.new(|cx| {
                    let builder = VideoPlayer::builder(source, gpui_media_backend::SystemBackend)
                        .options(VideoPlayerOptions::default());
                    builder
                        .build_in_window(window, cx)
                        .expect("failed to create WebDAV video player")
                });
                cx.subscribe(&player, |_, event, _| match event {
                    VideoPlayerEvent::StateChanged(state) => {
                        eprintln!("WebDAV playback state: {state:?}");
                        if let PlaybackState::Error(error) = state {
                            eprintln!("WebDAV playback failed: {error}");
                        }
                    }
                    VideoPlayerEvent::BufferingChanged(percent) => {
                        eprintln!("WebDAV buffering: {percent}%");
                    }
                    VideoPlayerEvent::FrameTransportChanged(transport) => {
                        eprintln!("WebDAV frame transport: {transport:?}");
                    }
                    _ => {}
                })
                .detach();
                player
            },
        );
        if let Err(error) = result {
            eprintln!("failed to open WebDAV example window: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });

    Ok(())
}
