use gpui::{ClipboardEntry, ClipboardFileOperation, ClipboardItem, ExternalPaths};
use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf, sync::Arc};

pub(crate) const URI_LIST: &str = "text/uri-list";
pub(crate) const GNOME_FILES: &str = "x-special/gnome-copied-files";
pub(crate) const KDE_CUT: &str = "application/x-kde-cutselection";
pub(crate) const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

/// One immutable selection snapshot, shared with every request for that source.
#[derive(Clone, Default)]
pub(crate) struct ClipboardSource(pub Vec<(&'static str, Arc<[u8]>)>);

impl ClipboardSource {
    pub fn new(item: &ClipboardItem) -> anyhow::Result<Self> {
        let mut data = Vec::new();
        if let Some(files) = item.files() {
            let uris = ExternalPaths(files.paths().iter().cloned().collect()).to_uri_list()?;
            let cut = files.operation() == ClipboardFileOperation::Cut;
            // Nautilus requires LF delimiters and no trailing empty line.
            let mut gnome = if cut {
                b"cut".to_vec()
            } else {
                b"copy".to_vec()
            };
            for uri in uris
                .split(|&byte| byte == b'\n')
                .filter(|line| !line.is_empty())
            {
                gnome.push(b'\n');
                gnome.extend_from_slice(uri.strip_suffix(b"\r").unwrap_or(uri));
            }
            data.push((GNOME_FILES, Arc::from(gnome)));
            data.push((URI_LIST, Arc::from(uris)));
            data.push((KDE_CUT, Arc::from(if cut { &b"1"[..] } else { &b"0"[..] })));
        } else {
            anyhow::ensure!(
                !item.entries.iter().any(|entry| matches!(
                    entry,
                    ClipboardEntry::Files(_) | ClipboardEntry::ExternalPaths(_)
                )),
                "invalid file clipboard paths"
            );
        }
        for entry in &item.entries {
            if let ClipboardEntry::Image(image) = entry {
                let mime = image.format().mime_type();
                if !data.iter().any(|(existing, _)| *existing == mime) {
                    data.push((mime, Arc::from(image.bytes())));
                }
            }
        }
        let has_text = item
            .entries
            .iter()
            .any(|entry| matches!(entry, ClipboardEntry::String(_)));
        if let Some(text) = item.text().or_else(|| has_text.then(String::new)) {
            let bytes: Arc<[u8]> = text.into_bytes().into();
            data.extend(TEXT_TYPES.into_iter().map(|mime| (mime, bytes.clone())));
        }
        Ok(Self(data))
    }

    #[cfg(feature = "wayland")]
    pub fn get(&self, mime: &str) -> Option<Arc<[u8]>> {
        self.0
            .iter()
            .find(|(name, _)| *name == mime)
            .map(|(_, bytes)| bytes.clone())
    }
}

pub(crate) fn decode_files(
    mime: &str,
    bytes: &[u8],
    cut: Option<&[u8]>,
) -> anyhow::Result<ClipboardItem> {
    let (operation, uris) = if mime == GNOME_FILES {
        let delimiter = bytes
            .iter()
            .position(|&byte| byte == b'\n')
            .ok_or_else(|| anyhow::anyhow!("missing file clipboard operation"))?;
        let (mode, rest) = (&bytes[..delimiter], &bytes[delimiter + 1..]);
        let operation = match mode.strip_suffix(b"\r").unwrap_or(mode) {
            b"copy" => ClipboardFileOperation::Copy,
            b"cut" => ClipboardFileOperation::Cut,
            _ => anyhow::bail!("invalid file clipboard operation"),
        };
        (operation, rest)
    } else {
        anyhow::ensure!(mime == URI_LIST, "unsupported file clipboard format");
        (
            if cut == Some(b"1".as_slice()) {
                ClipboardFileOperation::Cut
            } else {
                ClipboardFileOperation::Copy
            },
            bytes,
        )
    };
    let mut paths = Vec::new();
    for line in uris.split(|&byte| byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        let uri = line
            .strip_prefix(b"file://")
            .ok_or_else(|| anyhow::anyhow!("clipboard contains a non-local URI"))?;
        let path = uri
            .strip_prefix(b"localhost/")
            .map(|path| (&b"/"[..], path))
            .or_else(|| uri.starts_with(b"/").then_some((&b""[..], uri)))
            .ok_or_else(|| anyhow::anyhow!("clipboard contains a remote file URI"))?;
        let mut decoded = path.0.to_vec();
        let mut encoded = path.1.iter().copied();
        while let Some(byte) = encoded.next() {
            if byte == b'%' {
                let hex = |byte: Option<u8>| byte.and_then(|byte| (byte as char).to_digit(16));
                let high =
                    hex(encoded.next()).ok_or_else(|| anyhow::anyhow!("invalid URI escape"))?;
                let low =
                    hex(encoded.next()).ok_or_else(|| anyhow::anyhow!("invalid URI escape"))?;
                decoded.push((high * 16 + low) as u8);
            } else {
                anyhow::ensure!(
                    byte > b' ' && byte < 127 && !b"?#".contains(&byte),
                    "invalid file URI character"
                );
                decoded.push(byte);
            }
        }
        paths.push(PathBuf::from(OsString::from_vec(decoded)));
    }
    ClipboardItem::new_files(paths, operation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Image, ImageFormat};

    fn representation(source: &ClipboardSource, mime: &str) -> Arc<[u8]> {
        source
            .0
            .iter()
            .find(|(name, _)| *name == mime)
            .unwrap()
            .1
            .clone()
    }

    #[test]
    fn file_clipboard_exports_gnome_and_kde_copy_and_cut() {
        for operation in [ClipboardFileOperation::Copy, ClipboardFileOperation::Cut] {
            let paths = vec![
                PathBuf::from("/tmp/link/../中文 #%.png"),
                PathBuf::from(OsString::from_vec(b"/tmp/\xff\r\n".to_vec())),
            ];
            let item = ClipboardItem::new_files(paths.clone(), operation).unwrap();
            let source = ClipboardSource::new(&item).unwrap();
            let uri = representation(&source, URI_LIST);
            assert_eq!(
                &*uri,
                b"file:///tmp/link/../%E4%B8%AD%E6%96%87%20%23%25.png\r\nfile:///tmp/%FF%0D%0A\r\n"
            );
            let gnome = representation(&source, GNOME_FILES);
            let prefix: &[u8] = if operation == ClipboardFileOperation::Cut {
                b"cut\n"
            } else {
                b"copy\n"
            };
            assert!(gnome.starts_with(prefix));
            assert!(!gnome.ends_with(b"\n"));
            assert!(!gnome.contains(&b'\r'));
            let kde = representation(&source, KDE_CUT);
            for imported in [
                decode_files(GNOME_FILES, &gnome, None).unwrap(),
                decode_files(URI_LIST, &uri, Some(&kde)).unwrap(),
            ] {
                assert_eq!(imported.files().unwrap().paths(), paths);
                assert_eq!(imported.files().unwrap().operation(), operation);
            }
        }
    }

    #[test]
    fn file_clipboard_imports_local_uris_without_normalizing_paths() {
        let item = decode_files(
            URI_LIST,
            b"# comment\r\nfile://localhost/tmp/../a%20b\r\nfile:///tmp/%ff\n",
            None,
        )
        .unwrap();
        let files = item.files().unwrap();
        assert_eq!(files.operation(), ClipboardFileOperation::Copy);
        assert_eq!(
            files.paths(),
            [
                PathBuf::from("/tmp/../a b"),
                PathBuf::from(OsString::from_vec(b"/tmp/\xff".to_vec()))
            ]
        );
        let gnome = decode_files(GNOME_FILES, b"cut\nfile:///tmp/a", Some(b"0")).unwrap();
        assert_eq!(
            gnome.files().unwrap().operation(),
            ClipboardFileOperation::Cut
        );
    }

    #[test]
    fn file_clipboard_rejects_partial_or_ambiguous_file_lists() {
        for payload in [
            b"".as_slice(),
            b"file://server/tmp/a",
            b"file:///tmp/a\nhttps://example.com/b",
            b"file:relative",
            b"file:///tmp/%",
            b"file:///tmp/%0g",
            b"file:///tmp/%00",
            b"file:///tmp/a#fragment",
            b"file:///tmp/a?query",
            b"file:///tmp/raw space",
        ] {
            assert!(
                decode_files(URI_LIST, payload, None).is_err(),
                "{payload:?}"
            );
        }
        assert!(decode_files(GNOME_FILES, b"move\nfile:///tmp/a", None).is_err());
        assert!(decode_files(GNOME_FILES, b"cut", None).is_err());
        assert!(decode_files("text/plain", b"file:///tmp/a", None).is_err());
    }

    #[test]
    fn file_clipboard_legacy_paths_and_mixed_operations_default_to_copy() {
        let legacy = ClipboardEntry::ExternalPaths(ExternalPaths(
            [PathBuf::from("/tmp/a")].into_iter().collect(),
        ));
        let mut item =
            ClipboardItem::new_files([PathBuf::from("/tmp/b")], ClipboardFileOperation::Cut)
                .unwrap();
        item.entries.push(legacy);
        let source = ClipboardSource::new(&item).unwrap();
        let imported = decode_files(GNOME_FILES, &representation(&source, GNOME_FILES), None)
            .unwrap()
            .files()
            .unwrap();
        assert_eq!(imported.operation(), ClipboardFileOperation::Copy);
        assert_eq!(
            imported.paths(),
            [PathBuf::from("/tmp/b"), PathBuf::from("/tmp/a")]
        );
        assert_eq!(item.text().unwrap(), "/tmp/b\n/tmp/a");
        let invalid = ClipboardItem::from(ClipboardEntry::ExternalPaths(ExternalPaths(
            [PathBuf::from("relative")].into_iter().collect(),
        )));
        assert!(ClipboardSource::new(&invalid).is_err());
    }

    #[test]
    fn clipboard_representations_preserve_empty_text_and_image_format() {
        let empty = ClipboardSource::new(&ClipboardItem::new_string(String::new())).unwrap();
        assert_eq!(empty.0.len(), TEXT_TYPES.len());
        assert!(representation(&empty, "text/plain").is_empty());
        let image = Image::from_bytes(ImageFormat::Jpeg, vec![1, 2, 3]);
        let source = ClipboardSource::new(&ClipboardItem::new_image(&image)).unwrap();
        assert_eq!(source.0.len(), 1);
        assert_eq!(&*representation(&source, "image/jpeg"), &[1, 2, 3]);
        let text = ClipboardSource::new(&ClipboardItem::new_string("/tmp/file".into())).unwrap();
        assert!(
            text.0
                .iter()
                .all(|(mime, _)| ![URI_LIST, GNOME_FILES, KDE_CUT].contains(mime))
        );
    }

    #[test]
    fn file_clipboard_snapshot_survives_later_copies() {
        let mut item =
            ClipboardItem::new_files([PathBuf::from("/tmp/first")], ClipboardFileOperation::Cut)
                .unwrap();
        let source = ClipboardSource::new(&item).unwrap();
        item =
            ClipboardItem::new_files([PathBuf::from("/tmp/second")], ClipboardFileOperation::Copy)
                .unwrap();
        let next = ClipboardSource::new(&item).unwrap();
        assert_eq!(
            &*representation(&source, GNOME_FILES),
            b"cut\nfile:///tmp/first"
        );
        assert_eq!(
            &*representation(&next, GNOME_FILES),
            b"copy\nfile:///tmp/second"
        );
    }
}
