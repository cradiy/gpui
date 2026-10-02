use crate::{ClipboardEntry, ClipboardItem, ExternalPaths};
use std::path::PathBuf;

/// The requested operation when pasting files. GPUI never modifies the files.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ClipboardFileOperation {
    /// Copy the source files.
    #[default]
    Copy,
    /// Move the source files when the receiving application pastes them.
    Cut,
}

/// Local files and their paste intent. Native file clipboard export is supported on Linux.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardFiles {
    paths: ExternalPaths,
    operation: ClipboardFileOperation,
}

impl ClipboardFiles {
    /// Accepts nonempty absolute paths without resolving symlinks or reading the filesystem.
    pub fn new(
        paths: impl IntoIterator<Item = PathBuf>,
        operation: ClipboardFileOperation,
    ) -> anyhow::Result<Self> {
        let paths = ExternalPaths(paths.into_iter().collect());
        anyhow::ensure!(
            !paths.0.is_empty(),
            "file clipboard requires at least one path"
        );
        for path in paths.paths() {
            anyhow::ensure!(path.is_absolute(), "file clipboard paths must be absolute");
            anyhow::ensure!(
                !path.as_os_str().as_encoded_bytes().contains(&0),
                "file clipboard paths cannot contain NUL"
            );
        }
        Ok(Self { paths, operation })
    }

    /// Original absolute paths, in clipboard order.
    pub fn paths(&self) -> &[PathBuf] {
        self.paths.paths()
    }

    /// The requested paste operation, not a notification of a completed move.
    pub fn operation(&self) -> ClipboardFileOperation {
        self.operation
    }
}

impl ClipboardItem {
    /// Creates a file clipboard item. The receiver performs the copy or move on paste.
    pub fn new_files(
        paths: impl IntoIterator<Item = PathBuf>,
        operation: ClipboardFileOperation,
    ) -> anyhow::Result<Self> {
        Ok(ClipboardEntry::Files(ClipboardFiles::new(paths, operation)?).into())
    }

    /// Returns file entries as one list. Legacy external paths have Copy intent.
    /// Mixed operations are treated as Copy to avoid an unintended move.
    pub fn files(&self) -> Option<ClipboardFiles> {
        let mut paths = Vec::new();
        let mut cut = true;
        for entry in &self.entries {
            match entry {
                ClipboardEntry::Files(files) => {
                    paths.extend_from_slice(files.paths());
                    cut &= files.operation() == ClipboardFileOperation::Cut;
                }
                ClipboardEntry::ExternalPaths(files) => {
                    paths.extend_from_slice(files.paths());
                    cut = false;
                }
                _ => {}
            }
        }
        ClipboardFiles::new(
            paths,
            if cut {
                ClipboardFileOperation::Cut
            } else {
                ClipboardFileOperation::Copy
            },
        )
        .ok()
    }
}

impl ExternalPaths {
    /// Encodes absolute local paths as CRLF-separated file URIs, without filesystem access.
    /// Unix filenames retain their original bytes, including non-UTF-8 sequences.
    #[cfg(any(unix, target_os = "windows"))]
    pub fn to_uri_list(&self) -> anyhow::Result<Vec<u8>> {
        encode_file_uri_list(self.paths())
    }
}

#[cfg(any(unix, target_os = "windows"))]
pub(crate) fn encode_file_uri_list(paths: &[PathBuf]) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(!paths.is_empty(), "file list requires at least one path");
    let mut bytes = Vec::new();
    for path in paths {
        anyhow::ensure!(path.is_absolute(), "file paths must be absolute");
        anyhow::ensure!(
            !path.as_os_str().as_encoded_bytes().contains(&0),
            "file paths cannot contain NUL"
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            bytes.extend_from_slice(b"file://");
            for &byte in path.as_os_str().as_bytes() {
                if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
                    bytes.push(byte);
                } else {
                    const HEX: &[u8] = b"0123456789ABCDEF";
                    bytes.extend_from_slice(&[
                        b'%',
                        HEX[(byte >> 4) as usize],
                        HEX[(byte & 15) as usize],
                    ]);
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            let url = http_client::Url::from_file_path(path)
                .map_err(|_| anyhow::anyhow!("invalid file path"))?;
            bytes.extend_from_slice(url.as_str().as_bytes());
        }
        bytes.extend_from_slice(b"\r\n");
    }
    Ok(bytes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    #[test]
    fn file_clipboard_rejects_invalid_paths_before_export() {
        for paths in [
            vec![],
            vec![PathBuf::from("relative")],
            vec![PathBuf::from(OsString::from_vec(b"/tmp/\0".to_vec()))],
        ] {
            assert!(ClipboardItem::new_files(paths, ClipboardFileOperation::Cut).is_err());
        }
        assert!(
            ClipboardItem::new_string("file:///tmp/file".into())
                .files()
                .is_none()
        );
    }
}
