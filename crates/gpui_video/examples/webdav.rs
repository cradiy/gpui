use std::{env, time::Duration};

use anyhow::{Context as _, Result, bail};
use gpui::{App, AppContext, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_video::{
    MediaSource, NetworkSourceOptions, PlaybackState, VideoPlayer, VideoPlayerEvent,
    VideoPlayerOptions,
};

fn main() -> Result<()> {
    let url = env::args().nth(1).context(
        "usage: GPUI_VIDEO_WEBDAV_USERNAME=user \
         GPUI_VIDEO_WEBDAV_PASSWORD=password \
         cargo run -p gpui_video --example webdav -- <direct WebDAV file URL>",
    )?;
    let username = env::var("GPUI_VIDEO_WEBDAV_USERNAME").ok();
    let password = env::var("GPUI_VIDEO_WEBDAV_PASSWORD").ok();
    if username.is_some() != password.is_some() {
        bail!("GPUI_VIDEO_WEBDAV_USERNAME and GPUI_VIDEO_WEBDAV_PASSWORD must be set together");
    }

    let mut network = NetworkSourceOptions::default()
        .with_user_agent("gpui-video-webdav-example/0.1")
        .with_timeout(Duration::from_secs(15))
        .with_retry_count(3)
        .with_retry_backoff(Duration::from_millis(250), Duration::from_secs(3))
        .with_buffer_duration(Duration::from_secs(5));
    if let (Some(username), Some(password)) = (username, password) {
        network = network.with_basic_auth(username, password);
    }

    let source = MediaSource::from_uri(url)?.with_network_options(network);
    let title = format!("gpui_video WebDAV · {}", source.display_name());

    gpui_video::init()?;
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
                    VideoPlayer::new_in_window(source, VideoPlayerOptions::default(), window, cx)
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
