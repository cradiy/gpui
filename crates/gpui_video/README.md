# gpui_video

`gpui_video` is a reusable GPUI video component backed by GStreamer. It keeps playback, decoding and frame extraction separate from player chrome so applications can build different interfaces on top of the same core.

## Public capabilities

- play, pause, stop and replay
- duration, position, progress and seekability
- accurate or keyframe seeking
- skip forward and backward
- step forward and backward by decoded frames
- playback rate, volume and mute controls
- timestamped current-frame access
- independent frame extraction for thumbnails and scrubbing
- state, timeline, buffering, frame, transport, rate and volume events
- cumulative decoded, delivered and dropped-frame statistics
- coded size, crop rectangle and pixel-aspect-ratio aware presentation geometry
- CPU, linear Linux DMA-BUF and native tiled NV12 DMA-BUF frame transport
- HTTP request headers, authentication, proxy, timeout and source retry options
- network buffering progress and an explicit host-controlled reload operation

## Create a player entity

```rust
let player = cx.new(|cx| {
    VideoPlayer::new_in_window(
        MediaSource::parse("/path/to/video.mp4")?,
        VideoPlayerOptions {
            autoplay: true,
            ..VideoPlayerOptions::default()
        },
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

Run the custom play/pause and timeline example with:

```sh
cargo run -p gpui_video --example overlay_controls -- /path/to/video.mp4
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
GPUI_VIDEO_WEBDAV_USERNAME='user' \
GPUI_VIDEO_WEBDAV_PASSWORD='password' \
cargo run -p gpui_video --example webdav -- \
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
    anyhow::Ok(())
})?;
```

Forward stepping uses GStreamer's frame-step event. Backward stepping performs an accurate seek using the current frame duration because compressed video cannot generally decode backward.

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
        VideoPlayerEvent::FrameReady(frame) => {
            // Inspect PTS, size or transport.
        }
        VideoPlayerEvent::FrameTransportChanged(transport) => {}
        VideoPlayerEvent::DmaBufImportFailed(reason) => {
            // The player has already started automatic CPU renegotiation.
        }
        VideoPlayerEvent::PlaybackRateChanged(rate) => {}
        VideoPlayerEvent::VolumeChanged { volume, muted } => {}
    }
    cx.notify();
});
```

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
let extractor = VideoFrameExtractor::new(source.clone())?;
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

An extractor owns one paused GStreamer pipeline and serializes requests on a worker thread. Reuse it for thumbnail strips or hover previews instead of creating one extractor per frame.

Remote frame extraction requires a seekable server response, normally HTTP
byte-range support. This does not restrict ordinary sequential playback, but a
server that only returns full `200 OK` bodies cannot provide arbitrary hover
frames efficiently; the extractor reports that seek failure to the host.

The exact-request queue is bounded to two requests by default, so request producers receive backpressure instead of growing the worker queue without limit. Configure `VideoFrameExtractorOptions::request_queue_capacity` when a different amount of backpressure is needed. Latest-only requests use a separate one-slot mailbox and never discard exact thumbnail requests.

Dropping the final extractor handle does not wait for an in-flight GStreamer seek timeout on the calling thread. The worker is notified to stop and releases its pipeline after the active request returns.

Requests beyond the video stream duration return the closest available frame before the end. `SeekMode::Accurate` is the default; use `frame_at_with_mode` or `frame_at_blocking_with_mode` when keyframe speed is preferred.

## DMA-BUF status

Use `VideoPlayer::new_in_window` to pass the active renderer's `GpuSpecs` into the decoder setup. `gpui_video` advertises only native NV12 modifiers that GPUI reports as sampleable with two memory planes. It preserves the GStreamer DMA-BUF object identity and maps both NV12 image planes to the same object when appropriate.

If GPUI reports `DmaBufImportStatus::Failed` after presentation, the player automatically restricts the appsink to CPU frames and seeks to the current position to force renegotiation. Linear NV12/BGRA/RGBA DMA-BUF remains available when native import is not supported.
