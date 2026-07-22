use std::{cell::Cell, rc::Rc, time::Duration};

use anyhow::Result;
use gpui::{
    App, AppContext, Bounds, Context, CursorStyle, Entity, IntoElement, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Render, Window, WindowBounds,
    WindowOptions, canvas, div, prelude::*, px, relative, rgba, size,
};
use gpui_video::{
    MediaSource, PlaybackState, SeekMode, VideoPlayer, VideoPlayerOptions, video_container,
};

struct VideoControlsDemo {
    player: Entity<VideoPlayer>,
    timeline_bounds: Rc<Cell<Bounds<Pixels>>>,
    scrub_progress: Option<f64>,
    dragging_timeline: bool,
}

impl VideoControlsDemo {
    fn new(player: Entity<VideoPlayer>, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&player, |_, _, _, cx| cx.notify()).detach();
        Self {
            player,
            timeline_bounds: Rc::new(Cell::new(Bounds::default())),
            scrub_progress: None,
            dragging_timeline: false,
        }
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        let result = self.player.update(cx, |player, cx| match player.state() {
            PlaybackState::Playing | PlaybackState::Loading => player.pause(cx),
            _ => player.play(cx),
        });
        if let Err(error) = result {
            eprintln!("gpui_video controls example: {error:#}");
        }
    }

    fn progress_at(&self, position: Pixels) -> f64 {
        let bounds = self.timeline_bounds.get();
        let width = f32::from(bounds.size.width).max(1.0);
        let offset = f32::from(position - bounds.origin.x);
        f64::from((offset / width).clamp(0.0, 1.0))
    }

    fn begin_scrub(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.dragging_timeline = true;
        self.scrub_progress = Some(self.progress_at(event.position.x));
        cx.notify();
    }

    fn update_scrub(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.dragging_timeline {
            self.scrub_progress = Some(self.progress_at(event.position.x));
            cx.notify();
        }
    }

    fn finish_scrub(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.dragging_timeline {
            return;
        }
        self.dragging_timeline = false;
        let progress = self.progress_at(event.position.x);
        self.scrub_progress = None;

        let duration = self.player.read(cx).duration();
        if let Some(duration) = duration {
            let target = duration.mul_f64(progress);
            let result = self.player.update(cx, |player, cx| {
                player.seek_to(target, SeekMode::Accurate, cx)
            });
            if let Err(error) = result {
                eprintln!("gpui_video controls example: {error:#}");
            }
        }
        cx.notify();
    }
}

impl Render for VideoControlsDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (timeline, playing) = {
            let player = self.player.read(cx);
            (
                player.timeline(),
                matches!(
                    player.state(),
                    PlaybackState::Playing | PlaybackState::Loading
                ),
            )
        };
        let progress = self
            .scrub_progress
            .or_else(|| timeline.progress())
            .unwrap_or_default()
            .clamp(0.0, 1.0) as f32;
        let position = self
            .scrub_progress
            .and_then(|progress| {
                timeline
                    .duration()
                    .map(|duration| duration.mul_f64(progress))
            })
            .unwrap_or_else(|| timeline.position());
        let time_label = format!(
            "{} / {}",
            format_time(position),
            timeline
                .duration()
                .map(format_time)
                .unwrap_or_else(|| "--:--".to_owned())
        );
        let timeline_bounds = self.timeline_bounds.clone();
        let controls = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .items_center()
            .gap_3()
            .p_4()
            .bg(rgba(0x0b0f18d9))
            .child(
                div()
                    .id("video-demo-playback-toggle")
                    .flex()
                    .items_center()
                    .justify_center()
                    .w(px(72.0))
                    .h(px(34.0))
                    .rounded_md()
                    .cursor(CursorStyle::PointingHand)
                    .bg(rgba(0xffffff24))
                    .text_color(gpui::white())
                    .child(if playing { "Pause" } else { "Play" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx))),
            )
            .child(
                div()
                    .id("video-demo-timeline")
                    .relative()
                    .flex_1()
                    .h(px(28.0))
                    .cursor(CursorStyle::PointingHand)
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .top(px(11.0))
                            .h(px(6.0))
                            .rounded_full()
                            .bg(rgba(0xffffff40)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(11.0))
                            .w(relative(progress))
                            .h(px(6.0))
                            .rounded_full()
                            .bg(rgba(0x5aa9ffff)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left(relative(progress))
                            .top(px(7.0))
                            .ml(px(-7.0))
                            .size(px(14.0))
                            .rounded_full()
                            .bg(gpui::white()),
                    )
                    .child(
                        canvas(
                            move |bounds, _, _| timeline_bounds.set(bounds),
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .inset_0(),
                    )
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::begin_scrub))
                    .on_mouse_move(cx.listener(Self::update_scrub))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::finish_scrub))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::finish_scrub)),
            )
            .child(
                div()
                    .w(px(112.0))
                    .text_color(gpui::white())
                    .text_right()
                    .child(time_label),
            );

        div()
            .size_full()
            .bg(rgba(0x080b10ff))
            .child(video_container(self.player.clone()).child(controls))
    }
}

fn format_time(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn main() -> Result<()> {
    let input = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!(
            "Usage: cargo run -p gpui_video --example overlay_controls -- <video file or URI>"
        );
        std::process::exit(2);
    });
    let source = MediaSource::parse(input)?;

    gpui_video::init()?;
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(680.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("gpui_video · custom overlay controls");
                let player = cx.new(|cx| {
                    VideoPlayer::new_in_window(source, VideoPlayerOptions::default(), window, cx)
                        .expect("failed to create video player")
                });
                cx.new(|cx| VideoControlsDemo::new(player, cx))
            },
        );
        if let Err(error) = result {
            eprintln!("gpui_video: failed to open controls example: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });

    Ok(())
}
