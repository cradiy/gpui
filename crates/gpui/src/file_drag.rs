use crate::{DragIconPolicy, DragSourceWindowPolicy, Modifiers};
use std::{path::PathBuf, sync::Arc};

/// A negotiated file operation. GPUI never modifies source files.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragAction {
    /// Copy files.
    Copy,
    /// Move files; the receiving application owns the filesystem operation.
    Move,
    /// Create links (unsupported by the Wayland core drag protocol).
    Link,
}

bitflags::bitflags! {
    /// Actions permitted by a native file drag source.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct DragActions: u8 {
        /// Allow copying.
        const COPY = 1;
        /// Allow moving.
        const MOVE = 2;
        /// Allow creating links where supported.
        const LINK = 4;
    }
}
impl DragActions {
    /// Whether this set permits an action.
    pub fn allows(self, action: DragAction) -> bool {
        self.contains(match action {
            DragAction::Copy => Self::COPY,
            DragAction::Move => Self::MOVE,
            DragAction::Link => Self::LINK,
        })
    }
}

/// Presentation and negotiation options for exporting an active typed drag.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SystemFileDragOptions {
    /// Native drag preview.
    pub icon: DragIconPolicy,
    /// Source window visibility during native dragging.
    pub source_window: DragSourceWindowPolicy,
    /// Actions offered to the target.
    pub allowed_actions: DragActions,
    /// Source preference where supported. Wayland delegates preference to the compositor and target.
    pub preferred_action: DragAction,
}
impl Default for SystemFileDragOptions {
    fn default() -> Self {
        Self {
            icon: DragIconPolicy::ActiveDragView,
            source_window: DragSourceWindowPolicy::KeepVisible,
            allowed_actions: DragActions::COPY | DragActions::MOVE,
            preferred_action: DragAction::Move,
        }
    }
}
impl SystemFileDragOptions {
    /// Selects a preference from modifiers without exceeding allowed actions.
    pub fn action_for_modifiers(self, modifiers: Modifiers) -> DragAction {
        if modifiers.control && self.allowed_actions.allows(DragAction::Copy) {
            DragAction::Copy
        } else if modifiers.shift && self.allowed_actions.allows(DragAction::Move) {
            DragAction::Move
        } else {
            self.preferred_action
        }
    }
}

/// Native drag failure without a confirmed successful drop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DragFailure {
    /// The target stopped responding.
    TimedOut,
    /// File list transfer failed.
    Transfer,
    /// A native protocol operation failed.
    Protocol,
}

/// Validated, cached file data for a platform drag backend.
#[derive(Clone, Debug)]
pub struct SystemFileDrag {
    uri_list: Arc<[u8]>,
    options: SystemFileDragOptions,
}
impl SystemFileDrag {
    /// Encodes absolute paths without resolving symlinks or accessing the filesystem.
    #[cfg(any(unix, target_os = "windows"))]
    pub fn new(paths: Arc<[PathBuf]>, options: SystemFileDragOptions) -> anyhow::Result<Self> {
        anyhow::ensure!(!paths.is_empty(), "a file drag requires at least one path");
        anyhow::ensure!(
            options.allowed_actions.allows(options.preferred_action),
            "preferred drag action must be allowed"
        );
        let mut bytes = Vec::new();
        for path in paths.iter() {
            anyhow::ensure!(path.is_absolute(), "file drag paths must be absolute");
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt;
                let path = path.as_os_str().as_bytes();
                anyhow::ensure!(!path.contains(&0), "file drag paths cannot contain NUL");
                bytes.extend_from_slice(b"file://");
                for &byte in path {
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
                    .map_err(|_| anyhow::anyhow!("invalid file drag path"))?;
                bytes.extend_from_slice(url.as_str().as_bytes());
            }
            bytes.extend_from_slice(b"\r\n");
        }
        Ok(Self {
            uri_list: bytes.into(),
            options,
        })
    }
    /// Native file paths are unavailable on this platform.
    #[cfg(not(any(unix, target_os = "windows")))]
    pub fn new(_paths: Arc<[PathBuf]>, _options: SystemFileDragOptions) -> anyhow::Result<Self> {
        anyhow::bail!("native file paths are unsupported by this platform")
    }
    /// Cached `text/uri-list` bytes.
    pub fn uri_list(&self) -> &Arc<[u8]> {
        &self.uri_list
    }
    /// Negotiation and presentation options.
    pub fn options(&self) -> SystemFileDragOptions {
        self.options
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    #[test]
    fn file_drag_preserves_path_bytes_and_spelling() {
        let paths = vec![
            PathBuf::from("/tmp/link/../中文 #%.png"),
            PathBuf::from(OsString::from_vec(b"/tmp/\xff\r\n".to_vec())),
        ];
        let data = SystemFileDrag::new(paths.into(), Default::default()).unwrap();
        assert_eq!(
            data.uri_list().as_ref(),
            b"file:///tmp/link/../%E4%B8%AD%E6%96%87%20%23%25.png\r\nfile:///tmp/%FF%0D%0A\r\n"
        );
    }
    #[test]
    fn file_drag_rejects_invalid_sources_and_actions() {
        for paths in [
            vec![],
            vec![PathBuf::from("relative")],
            vec![PathBuf::from(OsString::from_vec(b"/tmp/\0".to_vec()))],
        ] {
            assert!(SystemFileDrag::new(paths.into(), Default::default()).is_err());
        }
        let options = SystemFileDragOptions {
            allowed_actions: DragActions::COPY,
            ..Default::default()
        };
        assert!(SystemFileDrag::new(vec!["/tmp/file".into()].into(), options).is_err());
    }
}
