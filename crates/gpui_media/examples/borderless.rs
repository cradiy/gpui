use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, Context, DevicePixels, Entity, IntoElement, KeyBinding, MouseButton,
    Render, Size, Task, Window, WindowBounds, WindowDecorations, WindowOptions, actions, div,
    prelude::*, px, size,
};
use gpui_media::{
    MediaSource, VideoFrameExtractor, VideoPlayer, fit_video_window_bounds, fit_video_window_size,
};

actions!(borderless_video, [Quit]);

struct BorderlessVideo {
    player: Entity<VideoPlayer>,
    _display_fit_task: Task<()>,
}

impl BorderlessVideo {
    fn new(
        player: Entity<VideoPlayer>,
        video_size: Size<DevicePixels>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let display_fit_task = cx.spawn_in(window, async move |this, cx| {
            for _ in 0..60 {
                let Ok(fitted) = this.update_in(cx, |_, window, cx| {
                    let display = window
                        .display(cx)
                        .or_else(|| cx.primary_display())
                        .or_else(|| cx.displays().into_iter().next());
                    let Some(display) = display else {
                        return false;
                    };

                    let fitted = fit_video_window_bounds(video_size, display.as_ref());
                    if window.window_bounds().get_bounds().size != fitted.size {
                        window.resize(fitted.size);
                    }
                    true
                }) else {
                    return;
                };
                if fitted {
                    return;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        });

        Self {
            player,
            _display_fit_task: display_fit_task,
        }
    }
}

impl Render for BorderlessVideo {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(gpui::black())
            .on_mouse_down(MouseButton::Left, |_, window, _| {
                window.start_window_move();
            })
            .child(self.player.clone())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let input = std::env::args().nth(1).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run -p gpui_media --example borderless -- <video file or URI>",
        )
    })?;
    let source = MediaSource::parse(input)?;

    gpui_media_backend::SystemBackend::initialize()?;
    let initial_frame = VideoFrameExtractor::new(
        source.clone(),
        std::sync::Arc::new(gpui_media_backend::SystemBackend),
    )?
    .initial_frame_blocking()?;
    let video_size = initial_frame.display_size();
    let video_size = size(
        DevicePixels(video_size.width),
        DevicePixels(video_size.height),
    );
    let title = format!("gpui_media · {}", source.display_name());

    gpui_platform::application().run(move |cx: &mut App| {
        let display = cx
            .primary_display()
            .or_else(|| cx.displays().into_iter().next());
        let bounds = display.map_or_else(
            || fallback_window_bounds(video_size, cx),
            |display| fit_video_window_bounds(video_size, display.as_ref()),
        );

        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                app_id: Some("gpui-media-borderless".into()),
                titlebar: None,
                is_resizable: false,
                is_minimizable: false,
                window_decorations: Some(WindowDecorations::Client),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title(&title);
                let player = cx.new(|cx| {
                    VideoPlayer::builder(source, gpui_media_backend::SystemBackend)
                        .build_in_window(window, cx)
                        .expect("failed to create video player")
                });
                cx.new(|cx| BorderlessVideo::new(player, video_size, window, cx))
            },
        );

        if let Err(error) = result {
            eprintln!("gpui_media borderless example: failed to open window: {error:#}");
            cx.quit();
            return;
        }

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("escape", Quit, None),
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.activate(true);
    });

    Ok(())
}

fn fallback_window_bounds(video_size: Size<DevicePixels>, cx: &App) -> Bounds<gpui::Pixels> {
    let fitted = fit_video_window_size(video_size, 1.0, size(px(1280.0), px(720.0)));
    Bounds::centered(None, fitted, cx)
}
