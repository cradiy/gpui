use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

/// A media URI accepted by the playback backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaSource {
    uri: String,
    display_name: String,
}

impl MediaSource {
    /// Creates a source from an already encoded URI.
    pub fn from_uri(uri: impl Into<String>) -> Result<Self> {
        let uri = uri.into();
        let parsed = url::Url::parse(&uri).context("invalid media URI")?;
        let display_name = parsed
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|name| !name.is_empty())
            .unwrap_or(parsed.as_str())
            .to_owned();

        Ok(Self { uri, display_name })
    }

    /// Creates a file URI from a local path.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let canonical = path
            .canonicalize()
            .with_context(|| format!("media file does not exist: {}", path.display()))?;
        if !canonical.is_file() {
            bail!("media source is not a file: {}", canonical.display());
        }

        let uri = url::Url::from_file_path(&canonical)
            .map_err(|_| {
                anyhow::anyhow!("cannot convert path to file URI: {}", canonical.display())
            })?
            .into();
        let display_name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("video")
            .to_owned();

        Ok(Self { uri, display_name })
    }

    /// Treats inputs containing a URI scheme as URIs and all other inputs as
    /// local filesystem paths.
    pub fn parse(input: impl AsRef<str>) -> Result<Self> {
        let input = input.as_ref();
        match url::Url::parse(input) {
            Ok(url) if !url.scheme().is_empty() => Self::from_uri(url.to_string()),
            _ => Self::from_path(PathBuf::from(input)),
        }
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }
}

impl TryFrom<&Path> for MediaSource {
    type Error = anyhow::Error;

    fn try_from(path: &Path) -> Result<Self> {
        Self::from_path(path)
    }
}

impl TryFrom<PathBuf> for MediaSource {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> Result<Self> {
        Self::from_path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::MediaSource;

    #[test]
    fn parses_remote_uri() {
        let source = MediaSource::parse("https://example.com/media/movie.mp4").unwrap();
        assert_eq!(source.uri(), "https://example.com/media/movie.mp4");
        assert_eq!(source.display_name(), "movie.mp4");
    }

    #[test]
    fn rejects_missing_local_file() {
        let error = MediaSource::parse("this-video-does-not-exist.mp4").unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }
}
