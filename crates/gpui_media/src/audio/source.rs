use std::{fmt, io, sync::Arc};

#[cfg(not(target_family = "wasm"))]
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
#[cfg(not(target_family = "wasm"))]
use std::{
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};
#[cfg(not(target_family = "wasm"))]
use symphonia::core::{
    formats::probe::Hint,
    io::{MediaSource as SymphoniaMediaSource, ReadOnlySource},
};

use crate::{MediaError, MediaErrorKind, MediaRecovery, MediaResult, MediaSource};

/// Format hints for a caller-fed encoded audio stream.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AudioStreamHint {
    extension: Option<String>,
    mime_type: Option<String>,
}

impl AudioStreamHint {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_extension(mut self, extension: impl Into<String>) -> Self {
        self.extension = Some(extension.into().trim_start_matches('.').to_owned());
        self
    }

    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }
}

/// The encoded input consumed by [`crate::AudioPlayer`].
///
/// File and URI sources are reopenable. A caller-fed stream is one-shot and
/// may only be opened by one player.
#[derive(Clone)]
pub struct AudioSource {
    kind: AudioSourceKind,
    hint: AudioStreamHint,
}

#[derive(Clone)]
enum AudioSourceKind {
    Media(MediaSource),
    Stream(Arc<StreamState>),
}

struct StreamState {
    receiver: async_channel::Receiver<Arc<[u8]>>,
    #[cfg(not(target_family = "wasm"))]
    opened: AtomicBool,
}

impl fmt::Debug for AudioSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            AudioSourceKind::Media(source) => formatter
                .debug_struct("AudioSource")
                .field("media", source)
                .field("hint", &self.hint)
                .finish(),
            AudioSourceKind::Stream(_) => formatter
                .debug_struct("AudioSource")
                .field("stream", &"caller-fed")
                .field("hint", &self.hint)
                .finish(),
        }
    }
}

impl AudioSource {
    pub fn from_uri(uri: impl Into<String>) -> MediaResult<Self> {
        MediaSource::from_uri(uri).map(Self::from)
    }

    pub fn from_path(path: impl AsRef<std::path::Path>) -> MediaResult<Self> {
        MediaSource::from_path(path).map(Self::from)
    }

    pub fn parse(input: impl AsRef<str>) -> MediaResult<Self> {
        MediaSource::parse(input).map(Self::from)
    }

    /// Creates a bounded, caller-fed encoded audio stream.
    ///
    /// The writer applies backpressure once `capacity` chunks are pending.
    /// Dropping or closing the writer marks end-of-stream.
    pub fn stream(
        capacity: usize,
        hint: AudioStreamHint,
    ) -> MediaResult<(AudioStreamWriter, Self)> {
        if capacity == 0 {
            return Err(MediaError::invalid_input(
                "audio stream capacity must be greater than zero",
            ));
        }
        let (sender, receiver) = async_channel::bounded(capacity);
        Ok((
            AudioStreamWriter { sender },
            Self {
                kind: AudioSourceKind::Stream(Arc::new(StreamState {
                    receiver,
                    #[cfg(not(target_family = "wasm"))]
                    opened: AtomicBool::new(false),
                })),
                hint,
            },
        ))
    }

    pub fn is_caller_fed(&self) -> bool {
        matches!(self.kind, AudioSourceKind::Stream(_))
    }

    #[cfg(target_family = "wasm")]
    pub(crate) fn browser_source(&self) -> MediaResult<MediaSource> {
        match &self.kind {
            AudioSourceKind::Media(source) => Ok(source.clone()),
            AudioSourceKind::Stream(state) => {
                state.receiver.close();
                Err(MediaError::unsupported(
                    "caller-fed audio streams are unavailable in the browser player; use an HTTP(S), blob, or data URL",
                ))
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn open(&self) -> MediaResult<OpenedAudioSource> {
        match &self.kind {
            AudioSourceKind::Media(source) => open_media_source(source, &self.hint),
            AudioSourceKind::Stream(state) => {
                if state.opened.swap(true, Ordering::AcqRel) {
                    return Err(MediaError::invalid_input(
                        "an audio stream source may only be opened once",
                    ));
                }
                Ok(OpenedAudioSource {
                    source: Box::new(ReadOnlySource::new(ChunkReader::new(
                        state.receiver.clone(),
                    ))),
                    hint: symphonia_hint(&self.hint),
                    seekable: false,
                })
            }
        }
    }

    #[cfg(not(target_family = "wasm"))]
    pub(crate) fn cancel(&self) {
        if let AudioSourceKind::Stream(state) = &self.kind {
            state.receiver.close();
        }
    }
}

impl From<MediaSource> for AudioSource {
    fn from(source: MediaSource) -> Self {
        let hint = source
            .uri()
            .split(['?', '#'])
            .next()
            .and_then(|uri| uri.rsplit_once('.'))
            .map(|(_, extension)| AudioStreamHint::new().with_extension(extension))
            .unwrap_or_default();
        Self {
            kind: AudioSourceKind::Media(source),
            hint,
        }
    }
}

/// Producer handle for a bounded [`AudioSource::stream`].
#[derive(Clone, Debug)]
pub struct AudioStreamWriter {
    sender: async_channel::Sender<Arc<[u8]>>,
}

impl AudioStreamWriter {
    /// Queues one encoded chunk, waiting asynchronously for capacity.
    pub async fn write(&self, chunk: impl Into<Vec<u8>>) -> MediaResult<()> {
        let chunk = chunk.into();
        if chunk.is_empty() {
            return Ok(());
        }
        self.sender
            .send(Arc::from(chunk))
            .await
            .map_err(|_| MediaError::invalid_input("audio stream is closed"))
    }

    /// Queues one encoded chunk without waiting for capacity.
    pub fn try_write(&self, chunk: impl Into<Vec<u8>>) -> MediaResult<()> {
        let chunk = chunk.into();
        if chunk.is_empty() {
            return Ok(());
        }
        self.sender
            .try_send(Arc::from(chunk))
            .map_err(|error| match error {
                async_channel::TrySendError::Full(_) => MediaError::new(
                    MediaErrorKind::Io {
                        kind: io::ErrorKind::WouldBlock,
                    },
                    "audio stream input queue is full",
                    MediaRecovery::Retry,
                ),
                async_channel::TrySendError::Closed(_) => MediaError::new(
                    MediaErrorKind::Io {
                        kind: io::ErrorKind::BrokenPipe,
                    },
                    "audio stream is closed",
                    MediaRecovery::None,
                ),
            })
    }

    /// Marks the stream as complete. Already queued chunks remain readable.
    pub fn close(&self) {
        self.sender.close();
    }

    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}

#[cfg(not(target_family = "wasm"))]
mod native {
    use super::*;
    pub(crate) struct OpenedAudioSource {
        pub(crate) source: Box<dyn SymphoniaMediaSource>,
        pub(crate) hint: Hint,
        pub(crate) seekable: bool,
    }

    pub(super) fn open_media_source(
        source: &MediaSource,
        fallback_hint: &AudioStreamHint,
    ) -> MediaResult<OpenedAudioSource> {
        let uri = url::Url::parse(source.uri()).map_err(|error| {
            MediaError::from_error(
                MediaErrorKind::InvalidInput,
                MediaRecovery::None,
                "invalid audio URI",
                error,
            )
        })?;
        match uri.scheme() {
            "file" => {
                let path = uri.to_file_path().map_err(|_| {
                    MediaError::invalid_input("cannot convert audio URI to a local path")
                })?;
                let file = std::fs::File::open(&path).map_err(|error| {
                    MediaError::io(format!("failed to open {}", path.display()), error)
                })?;
                Ok(OpenedAudioSource {
                    source: Box::new(file),
                    hint: symphonia_hint(fallback_hint),
                    seekable: true,
                })
            }
            "http" | "https" => open_http_source(source, fallback_hint),
            scheme => Err(MediaError::unsupported(format!(
                "audio source URI scheme {scheme:?} is not supported"
            ))),
        }
    }

    fn open_http_source(
        source: &MediaSource,
        fallback_hint: &AudioStreamHint,
    ) -> MediaResult<OpenedAudioSource> {
        let options = source.network_options();
        let mut agent = ureq::Agent::config_builder();
        if let Some(timeout) = options.timeout() {
            agent = agent.timeout_global(Some(timeout));
        }
        if let Some(proxy) = options.proxy() {
            let proxy = ureq::Proxy::new(proxy).map_err(|error| {
                MediaError::from_error(
                    MediaErrorKind::InvalidInput,
                    MediaRecovery::None,
                    "invalid audio proxy configuration",
                    error,
                )
            })?;
            agent = agent.proxy(Some(proxy));
        }
        let agent: ureq::Agent = agent.build().into();
        let mut request = agent.get(source.uri());
        for (name, value) in options.headers() {
            request = request.header(name, value);
        }
        if let Some(user_agent) = options.user_agent()
            && !options
                .headers()
                .keys()
                .any(|name| name.eq_ignore_ascii_case("user-agent"))
        {
            request = request.header("User-Agent", user_agent);
        }
        if !options
            .headers()
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization"))
            && (options.user_id().is_some() || options.user_password().is_some())
        {
            let credentials = format!(
                "{}:{}",
                options.user_id().unwrap_or_default(),
                options.user_password().unwrap_or_default()
            );
            request = request.header(
                "Authorization",
                format!("Basic {}", BASE64_STANDARD.encode(credentials)),
            );
        }
        let response = request.call().map_err(|error| {
            MediaError::from_error(
                MediaErrorKind::Network { status: None },
                MediaRecovery::Retry,
                format!("failed to open network audio {}", source.display_name()),
                source.redact_error_message(&error.to_string()),
            )
        })?;
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let (_, body) = response.into_parts();
        let mut hint = fallback_hint.clone();
        if hint.mime_type.is_none() {
            hint.mime_type = content_type;
        }
        Ok(OpenedAudioSource {
            source: Box::new(ReadOnlySource::new(body.into_reader())),
            hint: symphonia_hint(&hint),
            seekable: false,
        })
    }

    pub(super) fn symphonia_hint(source: &AudioStreamHint) -> Hint {
        let mut hint = Hint::new();
        if let Some(extension) = source.extension.as_deref() {
            hint.with_extension(extension);
        }
        if let Some(mime_type) = source.mime_type.as_deref() {
            hint.mime_type(mime_type);
        }
        hint
    }

    pub(super) struct ChunkReader {
        receiver: async_channel::Receiver<Arc<[u8]>>,
        current: Arc<[u8]>,
        offset: usize,
    }

    impl ChunkReader {
        pub(super) fn new(receiver: async_channel::Receiver<Arc<[u8]>>) -> Self {
            Self {
                receiver,
                current: Arc::from([]),
                offset: 0,
            }
        }
    }

    impl Read for ChunkReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            if output.is_empty() {
                return Ok(0);
            }
            while self.offset == self.current.len() {
                match self.receiver.recv_blocking() {
                    Ok(chunk) => {
                        self.current = chunk;
                        self.offset = 0;
                    }
                    Err(_) => return Ok(0),
                }
            }
            let length = output.len().min(self.current.len() - self.offset);
            output[..length].copy_from_slice(&self.current[self.offset..self.offset + length]);
            self.offset += length;
            Ok(length)
        }
    }
}
#[cfg(not(target_family = "wasm"))]
use native::*;

#[cfg(all(test, not(target_family = "wasm")))]
mod tests {
    use std::io::Read;

    use super::{AudioSource, AudioStreamHint};

    #[test]
    fn caller_fed_stream_is_bounded_and_one_shot() {
        let (writer, source) =
            AudioSource::stream(1, AudioStreamHint::new().with_extension("mp3")).unwrap();
        writer.try_write(vec![1, 2, 3]).unwrap();
        assert!(writer.try_write(vec![4]).is_err());
        writer.close();

        let mut opened = source.open().unwrap();
        let mut bytes = Vec::new();
        opened.source.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, [1, 2, 3]);
        assert!(source.open().is_err());
    }
}
