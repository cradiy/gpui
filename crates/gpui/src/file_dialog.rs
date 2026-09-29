use crate::BackgroundExecutor;
use anyhow::Result;
use futures::future::LocalBoxFuture;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// Options shared by desktop and browser file pickers.
#[derive(Clone, Debug, Default)]
pub struct FilePromptOptions {
    /// Allow more than one file to be selected.
    pub multiple: bool,
}

/// Platform storage for a selected file. Reads may require the platform UI thread.
pub trait PlatformFile: std::fmt::Debug + Send + Sync {
    /// Display name, including the extension.
    fn name(&self) -> &str;
    /// Native filesystem path, when available.
    fn path(&self) -> Option<&Path> {
        None
    }
    /// Browser URL, valid while the file handle is retained.
    fn url(&self) -> Option<&str> {
        None
    }
    /// Read the file on demand without blocking the UI thread.
    fn read(&self) -> LocalBoxFuture<'static, Result<Vec<u8>>>;
}

/// A selected file with lazy contents and shared ownership of its platform resource.
///
/// Browser files have a URL rather than a native path. Keep this handle alive
/// while a media player, image loader, or extractor uses that URL.
#[derive(Clone, Debug)]
pub struct SelectedFile(Arc<dyn PlatformFile>);

impl SelectedFile {
    /// Wrap a platform file resource.
    pub fn new(file: Arc<dyn PlatformFile>) -> Self {
        Self(file)
    }
    /// Display name, including the extension.
    pub fn name(&self) -> &str {
        self.0.name()
    }
    /// Native path, absent for browser files.
    pub fn path(&self) -> Option<&Path> {
        self.0.path()
    }
    /// Browser URL, valid while this handle or a clone is retained.
    pub fn url(&self) -> Option<&str> {
        self.0.url()
    }
    /// Read the contents asynchronously on the platform thread.
    pub fn read(&self) -> LocalBoxFuture<'static, Result<Vec<u8>>> {
        self.0.read()
    }

    pub(crate) fn from_path(path: PathBuf, executor: BackgroundExecutor) -> Self {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        Self::new(Arc::new(NativeFile {
            path,
            name,
            executor,
        }))
    }
}

struct NativeFile {
    path: PathBuf,
    name: String,
    executor: BackgroundExecutor,
}
impl std::fmt::Debug for NativeFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NativeFile").field(&self.path).finish()
    }
}
impl PlatformFile for NativeFile {
    fn name(&self) -> &str {
        &self.name
    }
    fn path(&self) -> Option<&Path> {
        Some(&self.path)
    }
    fn read(&self) -> LocalBoxFuture<'static, Result<Vec<u8>>> {
        let path = self.path.clone();
        Box::pin(self.executor.spawn(async move { Ok(std::fs::read(path)?) }))
    }
}
