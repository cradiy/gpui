# gpui_media

`gpui_media` is a reusable GPUI media playback core with pluggable backends.
One media session owns demuxing, video/audio decoding, audio output and the
shared playback clock. The [`gpui_media_system`](../gpui_media_system/README.md)
crate provides a platform-selected `SystemBackend`. Applications can also
inject their own backend implementation. Video frame rendering and
extraction remain separate from player chrome so applications can build
different interfaces on top of the same core.

Pure audio playback is provided separately by `AudioPlayer`. It uses the same
Symphonia and CPAL pipeline on every supported platform and does not depend on
`VideoPlayer` or a video backend. The default feature set enables
both audio and video. See the complete
[audio player guide](docs/audio-player.md).

## Public capabilities

- play, pause, stop and replay
- duration, position, progress and seekability
- accurate or keyframe seeking
- skip forward and backward
- step forward and backward by decoded frames
- playback rate, volume and mute controls
- unified audio/video lifecycle, timeline and playback clock
- audio and embedded-subtitle stream enumeration and selection
- normalized embedded subtitle cues for host rendering
- host-supplied SRT, WebVTT and ASS/SSA text parsing
- timestamped current-frame access
- independent frame extraction for thumbnails and scrubbing
- state, timeline, buffering, frame, transport, rate and volume events
- cumulative decoded, delivered and dropped-frame statistics
- coded size, crop rectangle and pixel-aspect-ratio aware presentation geometry
- CPU, macOS CoreVideo and Linux DMA-BUF frame transport
- HTTP request headers, authentication, proxy, timeout and source retry options
- network buffering progress and an explicit host-controlled reload operation
- standalone file, sequential HTTP and caller-fed encoded audio streams
- bounded asynchronous backpressure for caller-fed audio chunks

## Select media features and a playback backend

The default features enable audio and video APIs. Video playback requires a
backend; the core does not link GStreamer or Media Foundation.

```toml
gpui_media = { path = ".../gpui_media" }
gpui_media_system = { path = ".../gpui_media_system" }
```

For video without the independent audio player:

```toml
gpui_media = { path = ".../gpui_media", default-features = false, features = ["video"] }
gpui_media_system = { path = ".../gpui_media_system", features = ["v1_26"] }
```

`gpui_media_system::SystemBackend` selects GStreamer on Linux/macOS and Media
Foundation on Windows. GStreamer requires at least 1.24; version features and
runtime requirements are documented in the [backend guide](../gpui_media_system/README.md).

```rust
use gpui_media::{MediaSource, VideoPlayer};
use gpui_media_system::SystemBackend;

let player = VideoPlayer::builder(source, SystemBackend)
    .build_in_window(window, cx)?;
```

For audio without video, enable only `gpui_media`'s `audio` feature with
`default-features = false`; no backend crate is required.

## Custom backends

Implement `MediaBackend` to open a unified `MediaPlaybackSession` and,
optionally, a `FrameExtractionSession`. The session owns audio output and A/V
synchronization. It publishes timestamped `VideoFrame` values and playback
events through `MediaOutputSink`; `gpui_media` owns the bounded latest-video
queue and common frame statistics.

```rust
let backend: Arc<dyn MediaBackend> = Arc::new(MyBackend::new());
let player_source = source.clone();
let player_backend = backend.clone();

let player = cx.new(move |cx| {
    VideoPlayer::new_in_window(
        player_source,
        VideoPlayerOptions::default(),
        player_backend,
        window,
        cx,
    )
    .expect("failed to create video player")
});

let extractor = VideoFrameExtractor::new(source.clone(), backend)?;
```

Backends are selected per player. Multiple backend implementations may coexist.

## Media session boundary

`MediaPlaybackSession` owns all selected streams from one source. Audio and
video are intentionally not opened as independent playback sessions: play,
pause, seek, buffering, playback rate, EOS and clock synchronization must
remain atomic across the complete media timeline. The backend also owns audio
output; `MediaOutputSink` receives only decoded video frames and common media
events.

Backends emit `MediaBackendEvent::Ready` after initial preroll and after an
asynchronous seek completes. This lets an audio-only source leave
`PlaybackState::Loading` or `PlaybackState::Seeking` without waiting for a
video frame that will never exist.

## Create a player entity

```rust
let player = cx.new(|cx| {
    VideoPlayer::new_in_window(
        MediaSource::parse("/path/to/video.mp4")?,
        VideoPlayerOptions {
            autoplay: true,
            ..VideoPlayerOptions::default()
        },
        Arc::new(SystemBackend),
        window,
        cx,
    )
});
```

The entity itself implements `Render`, but deliberately paints only the current video frame. It does not install pointer handlers, draw status overlays, provide controls or manage fullscreen. Applications can wrap it with any interaction and control layout, or render `current_frame().surface()` directly when they need custom fitting and composition.

## Draw controls over the video container

`VideoContainer` gives the player and application-owned overlays the same
complete container bounds. The player uses `Contain` fitting inside it, while
overlay children may also use the letterbox area. The container exists before
the first frame arrives, so the host can provide its own loading UI. Passing
the player entity directly also lets GPUI refresh video frames independently.

```rust
let video = video_container(player.clone());

div().size_full().child(
    video.child(
        div()
            .absolute()
            .bottom_4()
            .left_4()
            .right_4()
            .child(custom_control_bar()),
    ),
)
```

The player only supplies the video-and-overlay container. Controls, pointer
behavior, subtitles, status overlays and fullscreen transitions remain owned
by the host application.

## Open a native-size borderless window

The first frame can be decoded before opening the native window, avoiding a
visible provisional-size resize. `fit_video_window_bounds` keeps the video's
one-device-pixel-to-one-device-pixel size when it fits; larger videos are
reduced proportionally to the display's visible area without cropping.

```rust
let source = MediaSource::parse(input)?;
let initial_frame = VideoFrameExtractor::new(source.clone(), Arc::new(SystemBackend))?
    .initial_frame_blocking()?;
let video_size = initial_frame.display_size();

gpui_platform::application().run(move |cx: &mut App| {
    let display = cx
        .primary_display()
        .or_else(|| cx.displays().into_iter().next())
        .expect("no display available");
    let bounds = fit_video_window_bounds(video_size, display.as_ref());

    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            window_decorations: Some(WindowDecorations::Client),
            is_resizable: false,
            ..Default::default()
        },
        move |window, cx| {
            cx.new(|cx| {
                VideoPlayer::builder(source, SystemBackend).build_in_window(window, cx)
                    .expect("failed to create video player")
            })
        },
    )
    .expect("failed to open video window");
});
```

For network media this performs a preroll request before the playback pipeline
is created. Applications that already know the encoded display size can skip
the probe and call `fit_video_window_bounds` directly. The initial-frame path
does not seek, so WebDAV servers without HTTP byte-range support can still be
probed.

Run the complete borderless example with:

```sh
cargo run -p gpui_media_system --example borderless -- /path/to/video.mp4
```

The example marks the window as non-resizable. GPUI exposes that as equal
minimum and maximum `xdg_toplevel` sizes on Wayland, so compositors that
automatically float fixed-size windows, including niri, retain the fitted
video size. It also uses the application id `gpui-media-borderless` for
compositors where an explicit window rule is preferred.

The entire content area starts a native window move on a left-button press.
Press Escape to close the window. The example keeps the native video size when
it fits and reduces oversized video proportionally to the selected display's
visible area.

Run the custom play/pause and timeline example with:

```sh
cargo run -p gpui_media_system --example overlay_controls -- /path/to/video.mp4
```

## Read the timeline

```rust
let timeline = player.read(cx).timeline();
let position = timeline.position();
let duration = timeline.duration();
let progress = timeline.progress();
let seekable = timeline.is_seekable();
```

Duration can be `None` while a remote or live source is still loading.

## Network sources

HTTP(S), HLS and DASH URLs use the same `MediaSource` API as local files. The
network configuration is also reused by `VideoFrameExtractor`, so authenticated
hover previews do not need a separate request path.

```rust
let network = NetworkSourceOptions::default()
    .with_bearer_token(token)?
    .with_referer("https://app.example.com/")?
    .with_user_agent("MyPlayer/1.0")
    .with_timeout(Duration::from_secs(15))
    .with_retry_count(3)
    .with_retry_backoff(Duration::from_millis(250), Duration::from_secs(3))
    .with_buffer_duration(Duration::from_secs(5));

let source = MediaSource::from_uri("https://cdn.example.com/video.m3u8")?
    .with_network_options(network);
```

WebDAV file playback uses the same HTTP source. Give the player the direct
file URL rather than a collection URL; directory discovery and `PROPFIND`
remain the responsibility of the host application's WebDAV client.

```rust
let network = NetworkSourceOptions::default()
    .with_basic_auth(username, password);
let source = MediaSource::from_uri(webdav_file_url)?
    .with_network_options(network);
```

Run the dedicated WebDAV example with credentials supplied outside the command
line:

```sh
GPUI_MEDIA_WEBDAV_USERNAME='user' \
GPUI_MEDIA_WEBDAV_PASSWORD='password' \
cargo run -p gpui_media_system --example webdav -- \
  'https://dav.example.com/remote.php/dav/files/user/video.mp4'
```

Custom headers are applied to dynamically created adaptive-stream segment
sources as well as the initial manifest request. Header values and proxy URLs
are redacted from `Debug` output.

`retry_count` configures retry behavior implemented by the HTTP source plugin.
It does not silently loop after a fatal demuxer, decoder or pipeline error. The
host can observe `PlaybackState::Error`, apply its own retry policy, then ask
the existing player entity to perform one clean reload:

```rust
player.update(cx, |player, cx| player.reload(true, cx))?;
```

While a non-live stream reports buffering below 100%, the player temporarily
pauses the pipeline and resumes it only if playback was still requested. A
user pause during buffering is therefore never overridden by the backend.

## Playback controls

```rust
player.update(cx, |player, cx| {
    player.pause(cx)?;
    player.seek_to(Duration::from_secs(30), SeekMode::Accurate, cx)?;
    player.skip_forward(Duration::from_secs(10), SeekMode::KeyFrame, cx)?;
    player.skip_backward(Duration::from_secs(5), SeekMode::Accurate, cx)?;
    player.step_forward(1, cx)?;
    player.step_backward(1, cx)?;
    player.set_playback_rate(1.5, cx)?;
    player.set_volume(0.8, cx);
    player.set_muted(false, cx);
    Ok::<_, gpui_media::MediaError>(())
})?;
```

Forward stepping is delegated to the active backend. Backward stepping performs
an accurate seek using the current frame duration because compressed video
cannot generally decode backward.

## Subscribe to events

```rust
cx.subscribe(&player, |_, _, event, cx| {
    match event {
        VideoPlayerEvent::StateChanged(state) => {
            // Update play and pause controls.
        }
        VideoPlayerEvent::TimelineChanged(timeline) => {
            // Update the scrubber and time labels.
        }
        VideoPlayerEvent::BufferingChanged(percent) => {
            // The host decides whether and how to present buffering UI.
        }
        VideoPlayerEvent::MediaInfoChanged(info) => {
            // Refresh application-owned audio and subtitle controls.
        }
        VideoPlayerEvent::Subtitle(event) => {
            // Store or clear extracted cues for host rendering.
        }
        VideoPlayerEvent::FrameReady(frame) => {
            // Inspect PTS, size or transport.
        }
        VideoPlayerEvent::FrameTransportChanged(transport) => {}
        VideoPlayerEvent::DmaBufImportFailed(reason) => {
            // The player has already started automatic CPU renegotiation.
        }
        VideoPlayerEvent::PlaybackRateChanged(rate) => {}
        VideoPlayerEvent::VolumeChanged { volume, muted } => {}
        _ => {}
    }
    cx.notify();
});
```

## Audio tracks and subtitles

`SystemBackend` reports available audio and embedded-subtitle streams through
`MediaInfoChanged`. Applications choose streams through the player while the
backend preserves the shared playback clock.

Embedded subtitle cues arrive through `VideoPlayerEvent::Subtitle`. External
SRT, WebVTT and ASS/SSA text can be normalized with `parse_subtitles`; cue
selection, composition, styling and rendering remain application-owned.

## Access the current frame

```rust
let frame = player.read(cx).current_frame().cloned();
if let Some(frame) = frame {
    let surface = frame.surface();
    let timestamp = frame.timestamp();
    let frame_duration = frame.duration();
    let coded_size = frame.coded_size();
    let visible_rect = frame.visible_rect();
    let display_size = frame.display_size();
    let format = frame.format();
    let color_info = frame.color_info();
    let transport = frame.transport();
}
```

Playback diagnostics are available without subscribing to every frame:

```rust
let stats = player.read(cx).stats();
let decoded = stats.decoded_frames();
let delivered = stats.delivered_frames();
let dropped = stats.dropped_frames();
let drop_ratio = stats.drop_ratio();
```

The returned frame owns or leases all resources required by its `SurfaceFrame`. Keep the `Arc<VideoFrame>` alive while another subsystem needs the pixels.

## Extract a frame without changing playback

```rust
let extractor = VideoFrameExtractor::new(source.clone(), Arc::new(SystemBackend))?;
let frame = extractor
    .frame_at(Duration::from_secs(30))
    .await?;
```

Interactive scrubbers should use the latest-only API so a pending preview can
be superseded when the pointer moves again:

```rust
let frame = extractor
    .frame_at_latest(Duration::from_secs_f64(scrub_seconds))
    .await?;
```

When several latest-only calls overlap, the superseded caller receives an
error for which `error.is::<FrameExtractionSuperseded>()` is true. Applications
can ignore that case while continuing to surface decoder and I/O failures.

For non-async workers:

```rust
let frame = extractor.frame_at_blocking(Duration::from_secs(30))?;
```

An extractor owns one backend extraction session and serializes requests on a
worker thread. Reuse it for thumbnail strips or hover previews instead of
creating one extractor per frame.

Remote frame extraction requires a seekable server response, normally HTTP
byte-range support. This does not restrict ordinary sequential playback, but a
server that only returns full `200 OK` bodies cannot provide arbitrary hover
frames efficiently; the extractor reports that seek failure to the host.

The exact-request queue is bounded to two requests by default, so request producers receive backpressure instead of growing the worker queue without limit. Configure `VideoFrameExtractorOptions::request_queue_capacity` when a different amount of backpressure is needed. Latest-only requests use a separate one-slot mailbox and never discard exact thumbnail requests.

Dropping the final extractor handle does not wait for an in-flight backend seek
timeout on the calling thread. The worker is notified to stop and releases its
session after the active request returns.

Requests beyond the video stream duration return the closest available frame before the end. `SeekMode::Accurate` is the default; use `frame_at_with_mode` or `frame_at_blocking_with_mode` when keyframe speed is preferred.

## DMA-BUF status

Use `VideoPlayer::new_in_window` to pass the active renderer's `GpuSpecs` into the decoder setup. `gpui_media_system` advertises only native NV12 modifiers that GPUI reports as sampleable with two memory planes. It preserves the GStreamer DMA-BUF object identity and maps both NV12 image planes to the same object when appropriate.

If GPUI reports `DmaBufImportStatus::Failed` after presentation, the player automatically restricts the appsink to CPU frames and seeks to the current position to force renegotiation. Linear NV12/BGRA/RGBA DMA-BUF remains available when native import is not supported.
