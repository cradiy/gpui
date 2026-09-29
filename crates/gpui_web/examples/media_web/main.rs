use gpui::{
    App, ClipboardItem, Context, Entity, FilePromptOptions, SelectedFile, Subscription, Window,
    WindowOptions, div, prelude::*, px, rgb,
};
use gpui_3d::{Camera, Material, Mesh, Object, Scene, viewport3d};
use gpui_media::{
    AudioPlayer, AudioPlayerOptions, AudioSource, MediaSource, SeekMode, VideoFrameExtractor,
    VideoPlayer, VideoPlayerOptions, VideoSurface,
};
use gpui_media_backend::SystemBackend;
use std::{rc::Rc, sync::Arc, time::Duration};

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
    _file: Option<SelectedFile>,
}

struct Demo {
    video: Option<LoadedMedia<VideoPlayer>>,
    audio: Option<LoadedMedia<AudioPlayer>>,
    poster: Option<Arc<gpui::SurfaceFrame>>,
    poster_generation: u64,
    clipboard_status: String,
    yaw: f32,
    error: Option<String>,
}
impl Demo {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            video: None,
            audio: None,
            poster: None,
            poster_generation: 0,
            clipboard_status: "Copy or paste text with the buttons below.".into(),
            yaw: 0.6,
            error: None,
        };
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

    fn select(&mut self, kind: MediaKind, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_files(FilePromptOptions::default());
        cx.spawn(async move |this, cx| {
            let result = selection.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(files))) => {
                        if let Some(file) = files.into_iter().next() {
                            if let Some(url) = file.url() {
                                this.load(
                                    kind,
                                    url.to_owned(),
                                    file.name().to_owned(),
                                    Some(file),
                                    cx,
                                );
                            }
                        }
                    }
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => this.error = Some(error.to_string()),
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn load(
        &mut self,
        kind: MediaKind,
        uri: String,
        name: String,
        file: Option<SelectedFile>,
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
                self.poster = None;
                self.poster_generation += 1;
                let generation = self.poster_generation;
                let extractor = VideoFrameExtractor::new(source.clone(), Arc::new(SystemBackend));
                let retained_file = file.clone();
                cx.spawn(async move |this, cx| {
                    let result = match extractor {
                        Ok(extractor) => extractor
                            .initial_frame()
                            .await
                            .and_then(|frame| VideoSurface::new().set_frame(&frame)),
                        Err(error) => Err(error),
                    };
                    let _ = this.update(cx, |this, cx| {
                        if generation != this.poster_generation {
                            return;
                        }
                        match result {
                            Ok(poster) => this.poster = Some(poster),
                            Err(error) => this.error = Some(format!("Poster: {error}")),
                        }
                        cx.notify();
                    });
                    drop(retained_file);
                })
                .detach();
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
                    _file: file,
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
                    _file: file,
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
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.select(kind, cx);
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
        if let Some(poster) = &self.poster {
            root = root.child("Extracted first frame").child(
                gpui::surface(poster.clone())
                    .w(px(220.))
                    .h(px(124.))
                    .object_fit(gpui::ObjectFit::Contain),
            );
        }
        root = root
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        div()
                            .id("copy-text")
                            .p_2()
                            .bg(rgb(0x36485f))
                            .child("Copy text")
                            .on_click(cx.listener(|_, _, _, cx| {
                                let task = cx.write_to_clipboard_async(ClipboardItem::new_string(
                                    "GPUI clipboard".into(),
                                ));
                                cx.spawn(async move |this, cx| {
                                    let result = task.await;
                                    let _ = this.update(cx, |this, cx| {
                                        this.clipboard_status = result
                                            .map(|_| "Copied GPUI clipboard".into())
                                            .unwrap_or_else(|e| e.to_string());
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })),
                    )
                    .child(
                        div()
                            .id("paste-text")
                            .p_2()
                            .bg(rgb(0x36485f))
                            .child("Paste text")
                            .on_click(cx.listener(|_, _, _, cx| {
                                let task = cx.read_from_clipboard_async();
                                cx.spawn(async move |this, cx| {
                                    let result = task.await;
                                    let _ = this.update(cx, |this, cx| {
                                        this.clipboard_status = result
                                            .map(|item| {
                                                item.and_then(|item| item.text())
                                                    .unwrap_or_default()
                                            })
                                            .unwrap_or_else(|e| e.to_string());
                                        cx.notify();
                                    });
                                })
                                .detach();
                            })),
                    ),
            )
            .child(self.clipboard_status.clone());
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
