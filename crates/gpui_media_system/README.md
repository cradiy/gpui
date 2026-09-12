# gpui_media_system

A platform-selected backend for [gpui_media](../gpui_media/README.md).
`SystemBackend` uses GStreamer on Linux and macOS, and Media Foundation on
Windows. Applications use the same player and frame-extraction APIs on each
platform.

## Dependencies

```toml
gpui_media = { path = ".../gpui_media", default-features = false, features = ["video"] }
gpui_media_system = { path = ".../gpui_media_system" }
```

Linux and macOS require GStreamer development libraries at build time and
runtime libraries with the plugins needed by the input media. Windows uses the
operating system's Media Foundation installation and does not depend on
GStreamer.

## GStreamer version

| Feature | Minimum system GStreamer version |
| --- | --- |
| `v1_24` (default) | 1.24 |
| `v1_26` | 1.26 |
| `v1_28` | 1.28 |

Version features are cumulative API levels for one set of GStreamer libraries.
Enabling more than one selects the highest minimum requirement; it does not
load multiple library versions. The Rust bindings remain on the 0.25 release
series.

```toml
gpui_media_system = { path = ".../gpui_media_system", features = ["v1_28"] }
```

At least one version feature must be enabled on Linux/macOS. The same features
have no effect on Windows: all GStreamer dependencies are target-specific.

## Playback

```rust
use gpui_media::{MediaSource, VideoPlayer, VideoPlayerOptions};
use gpui_media_system::SystemBackend;

let source = MediaSource::parse("/path/to/video.mp4")?;
let player = cx.new(|cx| {
    VideoPlayer::builder(source, SystemBackend)
        .options(VideoPlayerOptions::default())
        .build_in_window(window, cx)
        .expect("failed to open video")
});
```

The backend initializes when a session opens. `SystemBackend::initialize()`
can be called earlier to check initialization explicitly.

The player exposes controls, events, stream selection and current frames
without imposing a UI. See the [media guide](../gpui_media/README.md) for
containers, subtitles, controls and timeline access.

## Frame extraction

```rust
use std::{sync::Arc, time::Duration};
use gpui_media::{MediaSource, VideoFrameExtractor};
use gpui_media_system::SystemBackend;

let extractor = VideoFrameExtractor::new(
    MediaSource::parse("/path/to/video.mp4")?,
    Arc::new(SystemBackend),
)?;
let frame = extractor.frame_at(Duration::from_secs(5)).await?;
```

Frame extraction owns an independent session and does not seek an active
player.

## Platform capabilities

Linux supports CPU frames and DMA-BUF transport, including renderer-gated
native NV12 modifiers. macOS can deliver CoreVideo frames and CPU frames.
Windows delivers CPU frames through Media Foundation and WIC. Decoder and
container support depend on the system's installed media components.

On Linux/macOS, network source options configure supported GStreamer source
properties. Windows rejects custom network options that Media Foundation does
not expose through this backend. Session capabilities report the operations
available for the opened source.

## Examples

```sh
cargo run -p gpui_media_system --example play -- /path/to/video.mp4
cargo run -p gpui_media_system --example borderless -- /path/to/video.mp4
cargo run -p gpui_media_system --example overlay_controls -- /path/to/video.mp4
cargo run -p gpui_media_system --example frame_at -- /path/to/video.mp4 5
cargo run -p gpui_media_system --example tracks_and_subtitles -- \
  crates/gpui_media_system/examples/assets/tracks_and_subtitles.mp4 \
  crates/gpui_media_system/examples/assets/tracks_and_subtitles_external.srt
```

The `webdav` example accepts a direct media URL and optional
`GPUI_MEDIA_WEBDAV_USERNAME` / `GPUI_MEDIA_WEBDAV_PASSWORD` environment
variables.

## GStreamer distribution

Applications are responsible for complying with the licenses of the exact
GStreamer libraries and plugins they distribute. In particular, an application
that bundles GStreamer should retain the applicable notices, include the LGPL
license, provide the corresponding source as required, and keep dynamically
linked libraries replaceable. Plugin licenses can be inspected with
`gst-inspect-1.0 <plugin-or-element>`. Applications that use a system-installed
GStreamer do not redistribute those system libraries, but should still document
the runtime dependency. See the
[GStreamer licensing guidance](https://gstreamer.freedesktop.org/documentation/frequently-asked-questions/licensing.html).
