# gpui_media_core

Media sources, sessions, decoded frames, subtitles, and frame extraction without
a GUI or rendering dependency.

- `MediaBackend` opens playback and frame-extraction sessions.
- `MediaOutputSink` publishes frames through a bounded latest-frame queue and
  media events through an independent channel.
- `VideoFrameExtractor` serializes extraction requests on a worker, with exact
  requests and a latest-only preview mailbox.
- `VideoFrame` carries presentation timestamps, an immutable `FrameBuffer`, and
  optional `VideoDecoderInfo` supplied by the backend.
- `FrameBuffer` validates coded dimensions, crop, display size, pixel format,
  color metadata, and plane layouts.

## Frame ownership

CPU `FramePlane` values share immutable bytes through `Arc<[u8]>`, with explicit
offsets and row strides. BGRA, RGBA, and NV12 formats are supported. Display
dimensions account for pixel aspect ratio; they need not equal the coded size.

Linux `DmaBufImage` retains owned file descriptors, object-to-plane layouts,
DRM modifiers, optional producer device information, and a producer lease.
macOS `CoreVideoHandle` retains an immutable pixel buffer. Native constructors
require completed producer writes and prevent allocation reuse until the last
consumer releases its reference.

`FrameOutputCapabilities` describes native layouts a consumer can import. It
does not configure a graphics device or select a hardware decoder.

## Integration

[`gpui_media_system`](../gpui_media_system/README.md) provides the platform
backend and re-exports these types. Applications can depend on that crate alone
for playback sessions or frame extraction.

[`gpui_media`](../gpui_media/README.md) provides GPUI player entities and
`VideoSurface`, which adapts frames to GPUI surfaces without copying CPU pixels.
Custom backends can depend on `gpui_media_core` directly and implement
`MediaBackend` without importing either consumer or system backend.
