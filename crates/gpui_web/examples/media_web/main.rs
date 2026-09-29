use gpui::{App, Context, Entity, Subscription, Window, WindowOptions, div, prelude::*, px, rgb};
use gpui_3d::{Camera, Material, Mesh, Object, Scene, viewport3d};
use gpui_media::{
    AudioPlayer, AudioPlayerOptions, AudioSource, MediaSource, SeekMode, VideoPlayer,
    VideoPlayerOptions,
};
use gpui_media_backend::SystemBackend;
use std::{rc::Rc, time::Duration};

mod file_picker;
use file_picker::{FilePicker, ObjectUrl};

#[derive(Clone, Copy)]
enum MediaKind {
    Video,
    Audio,
}
impl MediaKind {
    fn key(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Video => "Load video",
            Self::Audio => "Load audio",
        }
    }
}

struct LoadedMedia<T: 'static> {
    player: Entity<T>,
    name: String,
    _subscription: Subscription,
    _object_url: Option<ObjectUrl>,
}

struct Demo {
    video: Option<LoadedMedia<VideoPlayer>>,
    audio: Option<LoadedMedia<AudioPlayer>>,
    video_picker: Option<FilePicker>,
    audio_picker: Option<FilePicker>,
    yaw: f32,
    error: Option<String>,
}
impl Demo {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            video: None,
            audio: None,
            video_picker: None,
            audio_picker: None,
            yaw: 0.6,
            error: None,
        };
        for kind in [MediaKind::Video, MediaKind::Audio] {
            let entity = cx.entity().downgrade();
            let mut app = cx.to_async();
            match FilePicker::new(kind.key(), move |file| {
                let _ = entity.update(&mut app, |this, cx| {
                    match ObjectUrl::new(&file) {
                        Ok(url) => {
                            let source = url.as_str().to_owned();
                            this.load(kind, source, file.name(), Some(url), cx);
                        }
                        Err(error) => this.error = Some(format!("Cannot open file: {error:?}")),
                    }
                    cx.notify();
                });
            }) {
                Ok(picker) => match kind {
                    MediaKind::Video => this.video_picker = Some(picker),
                    MediaKind::Audio => this.audio_picker = Some(picker),
                },
                Err(error) => this.error = Some(format!("Cannot create file picker: {error:?}")),
            }
        }
        let location = web_sys::window().unwrap().location();
        let params =
            web_sys::UrlSearchParams::new_with_str(&location.search().unwrap_or_default()).unwrap();
        for kind in [MediaKind::Video, MediaKind::Audio] {
            let Some(value) = params.get(kind.key()) else {
                continue;
            };
            match web_sys::Url::new_with_base(&value, &location.href().unwrap()) {
                Ok(url) => this.load(kind, url.href(), value, None, cx),
                Err(error) => this.error = Some(format!("Invalid media URL: {error:?}")),
            }
        }
        this
    }

    fn load(
        &mut self,
        kind: MediaKind,
        uri: String,
        name: String,
        object_url: Option<ObjectUrl>,
        cx: &mut Context<Self>,
    ) {
        let source = match MediaSource::from_uri(uri) {
            Ok(source) => source,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        match kind {
            MediaKind::Video => {
                let player = cx.new(|cx| {
                    VideoPlayer::builder(source, SystemBackend)
                        .options(VideoPlayerOptions {
                            autoplay: false,
                            ..Default::default()
                        })
                        .build(cx)
                        .expect("browser video initialization")
                });
                if let Some(previous) = &self.video {
                    let _ = previous.player.update(cx, |player, cx| player.pause(cx));
                }
                let subscription = cx.subscribe(&player, |_, _, _, cx| cx.notify());
                self.video = Some(LoadedMedia {
                    player,
                    name,
                    _subscription: subscription,
                    _object_url: object_url,
                });
            }
            MediaKind::Audio => {
                let player = cx.new(|cx| {
                    AudioPlayer::builder(AudioSource::from(source))
                        .options(AudioPlayerOptions {
                            autoplay: false,
                            ..Default::default()
                        })
                        .build(cx)
                        .expect("browser audio initialization")
                });
                if let Some(previous) = &self.audio {
                    previous.player.update(cx, |player, cx| player.pause(cx));
                }
                let subscription = cx.subscribe(&player, |_, _, _, cx| cx.notify());
                self.audio = Some(LoadedMedia {
                    player,
                    name,
                    _subscription: subscription,
                    _object_url: object_url,
                });
            }
        }
        self.error = None;
    }
}
impl Render for Demo {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_media = self.video.is_some() || self.audio.is_some();
        let scene = Scene::new()
            .camera(Camera::orbit(self.yaw, 0.35, 4.))
            .object(Object::new(Mesh::cube(), Material::color(rgb(0x62b8ee))));
        let mut root = div()
            .id("media-demo")
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .p_4()
            .gap_3()
            .bg(rgb(0x18212d))
            .text_color(rgb(0xffffff))
            .child("GPUI · WebGPU / Video / Audio")
            .child(
                div().flex().gap_3().flex_shrink_0().children(
                    [MediaKind::Video, MediaKind::Audio]
                        .into_iter()
                        .enumerate()
                        .map(|(index, kind)| {
                            div()
                                .id(("load", index))
                                .px_4()
                                .py_2()
                                .rounded_md()
                                .bg(rgb(0x36485f))
                                .cursor_pointer()
                                .child(kind.label())
                                .on_click(cx.listener(move |this, _, _, _| {
                                    let picker = match kind {
                                        MediaKind::Video => &this.video_picker,
                                        MediaKind::Audio => &this.audio_picker,
                                    };
                                    if let Some(picker) = picker {
                                        picker.open();
                                    }
                                }))
                        }),
                ),
            )
            .child(if window.supports_scene3d() {
                "3D: available"
            } else {
                "3D: unavailable on this device"
            })
            .child(
                div()
                    .id("rotate")
                    .p_2()
                    .bg(rgb(0x36485f))
                    .cursor_pointer()
                    .child("Rotate cube")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.yaw += 0.4;
                        cx.notify();
                    })),
            )
            .child(
                viewport3d("scene", scene)
                    .w(px(440.))
                    .h(px(240.))
                    .flex_shrink_0(),
            );
        if let Some(loaded) = &self.video {
            let video = &loaded.player;
            root = root
                .child(loaded.name.clone())
                .child(
                    div()
                        .w(px(440.))
                        .h(px(248.))
                        .flex_shrink_0()
                        .rounded(px(20.))
                        .overflow_hidden()
                        .child(video.clone()),
                )
                .child(format!(
                    "Video: {:?} · {:.2}s · {:?}",
                    video.read(cx).state(),
                    video.read(cx).timeline().position().as_secs_f64(),
                    video.read(cx).frame_transport()
                ));
        }
        if let Some(loaded) = &self.audio {
            let audio = &loaded.player;
            root = root.child(format!(
                "{} · Audio: {:?} · {:.2}s",
                loaded.name,
                audio.read(cx).state(),
                audio.read(cx).position().as_secs_f64()
            ));
        }
        root.child(if has_media {
            "Media playback"
        } else {
            "No media loaded. Choose Load video or Load audio above to enable playback."
        })
        .child(
            div().flex().flex_shrink_0().gap_3().children(
                ["Play", "Pause", "Seek +2s"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, label)| {
                        div()
                            .id(("control", index))
                            .p_3()
                            .rounded_md()
                            .bg(rgb(if has_media { 0x36485f } else { 0x242f3e }))
                            .text_color(rgb(if has_media { 0xffffff } else { 0x8290a3 }))
                            .child(label)
                            .when(has_media, |button| {
                                button.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.error = None;
                                        if let Some(video) = &this.video {
                                            let result =
                                                video.player.update(cx, |player, cx| match index {
                                                    0 => player.play(cx),
                                                    1 => player.pause(cx),
                                                    _ => player.seek_to(
                                                        player
                                                            .timeline()
                                                            .target_after(Duration::from_secs(2)),
                                                        SeekMode::Accurate,
                                                        cx,
                                                    ),
                                                });
                                            if let Err(error) = result {
                                                this.error = Some(error.to_string());
                                            }
                                        }
                                        if let Some(audio) = &this.audio {
                                            let result =
                                                audio.player.update(cx, |player, cx| match index {
                                                    0 => player.play(cx),
                                                    1 => {
                                                        player.pause(cx);
                                                        Ok(())
                                                    }
                                                    _ => player.seek_to(
                                                        player
                                                            .timeline()
                                                            .target_after(Duration::from_secs(2)),
                                                        SeekMode::Accurate,
                                                        cx,
                                                    ),
                                                });
                                            if let Err(error) = result {
                                                this.error = Some(error.to_string());
                                            }
                                        }
                                        cx.notify();
                                    },
                                ))
                            })
                    }),
            ),
        )
        .child(self.error.clone().unwrap_or_else(|| {
            "Play controls loaded media. Rotate cube controls the 3D preview.".into()
        }))
    }
}
fn main() {
    console_error_panic_hook::set_once();
    gpui_web::init_logging();
    gpui::Application::with_platform(Rc::new(gpui_web::WebPlatform::new(false))).run(
        |cx: &mut App| {
            cx.open_window(WindowOptions::default(), |_, cx| cx.new(Demo::new))
                .unwrap();
        },
    );
}
