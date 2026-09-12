use std::{collections::HashMap, fs, sync::Arc};

use gpui::{
    App, AppContext, Bounds, Context, CursorStyle, Entity, IntoElement, Render, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, rgba, size,
};
use gpui_media::{
    MediaInfo, MediaSource, MediaStreamId, PlaybackState, SubtitleCue, SubtitleEvent, VideoPlayer,
    VideoPlayerEvent, parse_subtitles, video_container,
};

struct TracksAndSubtitlesDemo {
    player: Entity<VideoPlayer>,
    external_cues: Arc<[SubtitleCue]>,
    embedded_cues: HashMap<MediaStreamId, Vec<Arc<SubtitleCue>>>,
}

impl TracksAndSubtitlesDemo {
    fn new(
        player: Entity<VideoPlayer>,
        external_cues: Arc<[SubtitleCue]>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&player, |this, _, event, cx| {
            if let VideoPlayerEvent::Subtitle(event) = event {
                match event {
                    SubtitleEvent::Reset => this.embedded_cues.clear(),
                    SubtitleEvent::Cue { stream_id, cue } => this
                        .embedded_cues
                        .entry(stream_id.clone())
                        .or_default()
                        .push(cue.clone()),
                    _ => {}
                }
            }
            cx.notify();
        })
        .detach();

        Self {
            player,
            external_cues,
            embedded_cues: HashMap::new(),
        }
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        let result = self.player.update(cx, |player, cx| match player.state() {
            PlaybackState::Playing | PlaybackState::Loading => player.pause(cx),
            _ => player.play(cx),
        });
        if let Err(error) = result {
            eprintln!("playback control failed: {error}");
        }
    }

    fn select_next_audio_stream(&mut self, cx: &mut Context<Self>) {
        let next = self.player.read(cx).media_info().and_then(|info| {
            if info.audio_streams.is_empty() {
                return None;
            }
            let next = info
                .audio_streams
                .iter()
                .position(|stream| stream.selected)
                .map_or(0, |index| (index + 1) % info.audio_streams.len());
            Some(info.audio_streams[next].id.clone())
        });
        let Some(next) = next else {
            return;
        };
        if let Err(error) = self
            .player
            .update(cx, |player, cx| player.select_audio_stream(&next, cx))
        {
            eprintln!("audio stream selection failed: {error}");
        }
    }

    fn select_next_subtitle_stream(&mut self, cx: &mut Context<Self>) {
        let next = self.player.read(cx).media_info().map(|info| {
            let current = info
                .subtitle_streams
                .iter()
                .position(|stream| stream.selected)
                .map_or(0, |index| index + 1);
            let next = (current + 1) % (info.subtitle_streams.len() + 1);
            next.checked_sub(1)
                .map(|index| info.subtitle_streams[index].id.clone())
        });
        let Some(next) = next else {
            return;
        };
        if let Err(error) = self.player.update(cx, |player, cx| {
            player.select_subtitle_stream(next.as_ref(), cx)
        }) {
            eprintln!("subtitle stream selection failed: {error}");
        }
    }

    fn active_subtitle_text(
        &self,
        info: Option<&MediaInfo>,
        position: std::time::Duration,
    ) -> String {
        let embedded = info
            .and_then(MediaInfo::selected_subtitle_stream)
            .and_then(|stream| self.embedded_cues.get(&stream.id))
            .into_iter()
            .flatten()
            .map(Arc::as_ref);

        self.external_cues
            .iter()
            .chain(embedded)
            .filter(|cue| cue.start <= position && position < cue.end)
            .map(|cue| cue.text.as_ref())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Render for TracksAndSubtitlesDemo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let player = self.player.read(cx);
        let info = player.media_info();
        let playing = matches!(
            player.state(),
            PlaybackState::Playing | PlaybackState::Loading
        );
        let subtitle_text = self.active_subtitle_text(info, player.position());
        let has_subtitle_text = !subtitle_text.is_empty();
        let audio_label = info
            .and_then(MediaInfo::selected_audio_stream)
            .map(stream_label)
            .unwrap_or_else(|| "Audio: unavailable".to_owned());
        let subtitle_label = info
            .and_then(MediaInfo::selected_subtitle_stream)
            .map(stream_label)
            .unwrap_or_else(|| "Embedded subtitles: off".to_owned());
        let button = |id, label: String| {
            div()
                .id(id)
                .px_3()
                .py_2()
                .rounded_md()
                .cursor(CursorStyle::PointingHand)
                .bg(rgba(0xffffff24))
                .text_color(gpui::white())
                .child(label)
        };

        let controls = div()
            .absolute()
            .bottom_0()
            .left_0()
            .right_0()
            .flex()
            .items_center()
            .gap_3()
            .p_4()
            .bg(rgba(0x0b0f18df))
            .child(
                button(
                    "tracks-playback-toggle",
                    if playing { "Pause" } else { "Play" }.to_owned(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx))),
            )
            .child(
                button("tracks-audio-next", audio_label)
                    .on_click(cx.listener(|this, _, _, cx| this.select_next_audio_stream(cx))),
            )
            .child(
                button("tracks-subtitle-next", subtitle_label)
                    .on_click(cx.listener(|this, _, _, cx| this.select_next_subtitle_stream(cx))),
            );

        let subtitles = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(px(82.0))
            .flex()
            .justify_center()
            .child(
                div()
                    .max_w(px(900.0))
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgba(0x000000b8))
                    .text_color(gpui::white())
                    .text_size(px(26.0))
                    .text_center()
                    .child(subtitle_text),
            );

        div().size_full().bg(rgba(0x080b10ff)).child(
            video_container(self.player.clone())
                .when(has_subtitle_text, |video| video.child(subtitles))
                .child(controls),
        )
    }
}

fn stream_label(stream: &impl StreamLabel) -> String {
    stream.stream_label()
}

trait StreamLabel {
    fn stream_label(&self) -> String;
}

impl StreamLabel for gpui_media::AudioStreamInfo {
    fn stream_label(&self) -> String {
        format!(
            "Audio: {}",
            self.language
                .as_deref()
                .or(self.title.as_deref())
                .unwrap_or(self.id.as_str())
        )
    }
}

impl StreamLabel for gpui_media::SubtitleStreamInfo {
    fn stream_label(&self) -> String {
        format!(
            "Embedded subtitles: {}",
            self.language
                .as_deref()
                .or(self.title.as_deref())
                .unwrap_or(self.id.as_str())
        )
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let media = arguments.next().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "usage: cargo run -p gpui_media --example tracks_and_subtitles -- \
             <media file or URI> [external subtitle.srt|vtt|ass]",
        )
    })?;
    let source = MediaSource::parse(media)?;
    let external_cues = arguments
        .next()
        .map(|path| {
            // The host owns file access and decoding; gpui_media only parses text.
            let source_text = fs::read_to_string(path)?;
            Ok::<_, Box<dyn std::error::Error>>(parse_subtitles(&source_text, None)?.cues)
        })
        .transpose()?
        .unwrap_or_default();

    gpui_media_backend::SystemBackend::initialize()?;
    gpui_platform::application().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1100.0), px(680.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                window.set_window_title("gpui_media · tracks and subtitles");
                let player = cx.new(|cx| {
                    VideoPlayer::builder(source, gpui_media_backend::SystemBackend)
                        .build_in_window(window, cx)
                        .expect("failed to create video player")
                });
                cx.new(|cx| TracksAndSubtitlesDemo::new(player, external_cues, cx))
            },
        );
        if let Err(error) = result {
            eprintln!("failed to open tracks and subtitles example: {error}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });

    Ok(())
}
