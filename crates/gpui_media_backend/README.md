# gpui_media_backend

A renderer-independent media backend with no GPUI dependency.
`SystemBackend` uses GStreamer on Linux and macOS, and Media Foundation on
Windows. Applications use the same player and frame-extraction APIs on each
platform. Media types are re-exported from
[`gpui_media_core`](../gpui_media_core/README.md).

## Dependencies

```toml
gpui_media_backend = { path = ".../gpui_media_backend" }
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
gpui_media_backend = { path = ".../gpui_media_backend", features = ["v1_28"] }
```

At least one version feature must be enabled on Linux/macOS. The same features
have no effect on Windows: all GStreamer dependencies are target-specific.

## Playback sessions

```rust
use gpui_media_backend::{
    MediaBackend, MediaOutputSink, MediaPlaybackRequest, MediaSource, SystemBackend,
};

let (sink, output) = MediaOutputSink::channel();
let mut session = SystemBackend.open_playback(
    MediaPlaybackRequest {
        source: MediaSource::parse("/path/to/video.mp4")?,
        output_capabilities: None,
    },
    sink,
)?;
session.play()?;
let frame = output.video_frames.recv_blocking()?;
```

The session owns audio output and A/V synchronization. Consume both the frame
and event channels during playback; events include readiness, buffering,
stream metadata, subtitles, errors, and end of stream. CPU frames expose
immutable planes, offsets, and strides. Native frames retain their platform
allocations until consumers release them.

## GPUI playback

Add `gpui_media` with its `video` feature for a GPUI player entity:

```toml
gpui_media = { path = ".../gpui_media", default-features = false, features = ["video"] }
```

```rust
use gpui_media::{MediaSource, VideoPlayer, VideoPlayerOptions};
use gpui_media_backend::SystemBackend;

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
use gpui_media_backend::{MediaSource, VideoFrameExtractor, SystemBackend};

let extractor = VideoFrameExtractor::new(
    MediaSource::parse("/path/to/video.mp4")?,
    Arc::new(SystemBackend),
)?;
let frame = extractor.frame_at(Duration::from_secs(5)).await?;
```

Frame extraction owns an independent session and does not seek an active
player. It requires no window, application context, or rendering device.

For GStreamer, `VideoFrameExtractorOptions::timeout` is one shared waiting
budget per active extraction request, including initial preroll and seek
retries. Time spent queued behind another request is not included. Synchronous
plugin calls cannot be forcibly interrupted by this budget.

On Linux/macOS, `VideoFrameExtractorOptions::video_decoder` selects the video
decoder policy for each extraction session:

| Policy | Allowed video decoders |
| --- | --- |
| `Auto` (default) | System registry selection |
| `SoftwareOnly` | Decoders without the GStreamer `Hardware` classification |
| `HardwareOnly` | Decoders with the GStreamer `Hardware` classification |

```rust
use gpui_media_backend::{VideoDecoderPolicy, VideoFrameExtractorOptions};

let options = VideoFrameExtractorOptions {
    video_decoder: VideoDecoderPolicy::SoftwareOnly,
    ..Default::default()
};
let extractor = VideoFrameExtractor::with_options(source, options, backend)?;
```

Explicit policies filter candidates within the session and verify the observed
decoder before returning frames. They do not change global plugin ranks or
other sessions. Missing compatible decoders, unidentifiable decoders, or failed
output negotiation return errors; an excluded decoder is not a fallback.
The policy controls decoder classification, not a plugin's internal execution
or fallback behavior, and does not select a GPU device or frame transport.

Automatic extraction uses `playbin3`; explicit policies use `playbin` with
`uridecodebin` candidate filtering. Windows supports `Auto` and rejects explicit
policies with `MediaErrorKind::Unsupported`. Playback sessions use automatic
decoder selection independently of extraction options.

## Platform capabilities

Linux supports CPU frames and DMA-BUF transport, including consumer-advertised
native NV12 modifiers. macOS can deliver CoreVideo frames and CPU frames.
Windows delivers CPU frames through Media Foundation and WIC. Decoder and
container support depend on the system's installed media components.

`MediaPlaybackRequest::output_capabilities` describes native layouts accepted
by the consumer. It does not choose the decoder or its device. GStreamer
selects decoders from its plugin registry; CPU frame output does not imply
software decoding. Playback sessions do not expose an explicit hardware-decoder
selection policy; extraction policies are described above.

`VideoFrame::decoder_info()` exposes a snapshot of the observed video decoder.
GStreamer reports the factory name after its output pad produces raw video,
matched to the output stream. Acceleration follows the factory's `Hardware`
classification; it is not a measurement of GPU usage. Available device
properties use backend-specific names, such as `device-path` or `cuda-device-id`.
Metadata is absent when no unique decoder can be identified. Media Foundation
does not currently expose decoder metadata through this backend.

Decoder metadata is available on both playback and extracted frames. It is
independent of `VideoFrame::transport()`: a hardware decoder can deliver CPU
frames. Retained frames keep their metadata when the session changes or closes.

On Linux/macOS, network source options configure supported GStreamer source
properties. Windows rejects custom network options that Media Foundation does
not expose through this backend. Session capabilities report the operations
available for the opened source.

## Examples

```sh
cargo run -p gpui_media_backend --example frame_at -- /path/to/video.mp4 5
cargo run -p gpui_media_backend --example frame_at -- /path/to/video.mp4 5 software
```

GUI examples live in `gpui_media`:

```sh
cargo run -p gpui_media --example play -- /path/to/video.mp4
cargo run -p gpui_media --example tracks_and_subtitles -- \
  crates/gpui_media/examples/assets/tracks_and_subtitles.mp4 \
  crates/gpui_media/examples/assets/tracks_and_subtitles_external.srt
```

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
