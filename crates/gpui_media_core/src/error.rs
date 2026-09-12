use std::{error::Error, fmt, io};

use std::sync::Arc;

/// A backend-independent category for media failures.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaErrorKind {
    InvalidInput,
    Authentication,
    Network { status: Option<u16> },
    Io { kind: io::ErrorKind },
    SourceNotFound,
    Timeout,
    Superseded,
    UnsupportedContainer,
    UnsupportedCodec,
    UnsupportedOperation,
    Decode,
    AudioOutput,
    VideoOutput,
    Backend,
}

/// The action a host can reasonably offer after a media failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum MediaRecovery {
    /// Repeating the operation is not expected to succeed.
    #[default]
    None,
    /// Repeating the operation or reloading the same source may succeed.
    Retry,
    /// The source must be recreated, for example with refreshed credentials.
    ReloadSource,
}

/// A structured, presentation-independent media failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MediaError {
    pub kind: MediaErrorKind,
    pub message: Arc<str>,
    pub recovery: MediaRecovery,
}

impl MediaError {
    pub fn new(
        kind: MediaErrorKind,
        message: impl Into<Arc<str>>,
        recovery: MediaRecovery,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            recovery,
        }
    }

    pub fn backend(message: impl Into<Arc<str>>) -> Self {
        Self::new(MediaErrorKind::Backend, message, MediaRecovery::None)
    }

    pub fn backend_retryable(message: impl Into<Arc<str>>) -> Self {
        Self::new(MediaErrorKind::Backend, message, MediaRecovery::Retry)
    }

    pub fn invalid_input(message: impl Into<Arc<str>>) -> Self {
        Self::new(MediaErrorKind::InvalidInput, message, MediaRecovery::None)
    }

    pub fn timeout(message: impl Into<Arc<str>>) -> Self {
        Self::new(MediaErrorKind::Timeout, message, MediaRecovery::Retry)
    }

    pub fn io(context: impl fmt::Display, error: io::Error) -> Self {
        let kind = error.kind();
        Self::new(
            if kind == io::ErrorKind::NotFound {
                MediaErrorKind::SourceNotFound
            } else {
                MediaErrorKind::Io { kind }
            },
            format!("{context}: {error}"),
            if matches!(
                kind,
                io::ErrorKind::Interrupted
                    | io::ErrorKind::TimedOut
                    | io::ErrorKind::WouldBlock
                    | io::ErrorKind::ConnectionAborted
                    | io::ErrorKind::ConnectionReset
            ) {
                MediaRecovery::Retry
            } else {
                MediaRecovery::None
            },
        )
    }

    pub fn from_error(
        kind: MediaErrorKind,
        recovery: MediaRecovery,
        context: impl fmt::Display,
        error: impl fmt::Display,
    ) -> Self {
        Self::new(kind, format!("{context}: {error}"), recovery)
    }

    pub fn unsupported(message: impl Into<Arc<str>>) -> Self {
        Self::new(
            MediaErrorKind::UnsupportedOperation,
            message,
            MediaRecovery::None,
        )
    }

    pub fn retryable(&self) -> bool {
        self.recovery != MediaRecovery::None
    }
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl Error for MediaError {}

pub type MediaResult<T> = Result<T, MediaError>;

#[cfg(test)]
mod tests {
    use super::{MediaError, MediaErrorKind, MediaRecovery};

    #[test]
    fn recovery_controls_retryability() {
        let terminal = MediaError::new(
            MediaErrorKind::UnsupportedCodec,
            "codec is unavailable",
            MediaRecovery::None,
        );
        let transient = MediaError::new(
            MediaErrorKind::Network { status: Some(503) },
            "service is unavailable",
            MediaRecovery::Retry,
        );

        assert!(!terminal.retryable());
        assert!(transient.retryable());
        assert_eq!(terminal.to_string(), "codec is unavailable");
    }
}
