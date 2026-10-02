use std::{
    cell::Cell,
    fs::File,
    io::{ErrorKind, Write},
    os::fd::{AsRawFd, BorrowedFd, OwnedFd},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use calloop::{LoopHandle, PostAction};
use filedescriptor::Pipe;
use strum::IntoEnumIterator;
use wayland_client::{Connection, protocol::wl_data_offer::WlDataOffer};
use wayland_protocols::wp::primary_selection::zv1::client::zwp_primary_selection_offer_v1::ZwpPrimarySelectionOfferV1;

use crate::linux::{
    WaylandClientStatePtr,
    clipboard::{ClipboardSource, GNOME_FILES, KDE_CUT, URI_LIST, decode_files},
    platform::{PIPE_READ_TIMEOUT, read_fd_with_timeout},
};
use gpui::{ClipboardEntry, ClipboardItem, Image, ImageFormat, hash};

/// File list MIME type shared with native drag-and-drop.
pub(crate) const FILE_LIST_MIME_TYPE: &str = "text/uri-list";

/// Text mime types that we'll accept from other programs.
pub(crate) const ALLOWED_TEXT_MIME_TYPES: [&str; 3] =
    ["text/plain;charset=utf-8", "UTF8_STRING", "text/plain"];

pub(crate) struct Clipboard {
    connection: Connection,
    loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    self_mime: String,

    // Internal clipboard
    contents: Option<ClipboardItem>,
    primary_contents: Option<ClipboardItem>,

    // External clipboard
    cached_read: Option<ClipboardItem>,
    current_offer: Option<DataOffer<WlDataOffer>>,
    cached_primary_read: Option<ClipboardItem>,
    current_primary_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>,
}

pub(crate) trait ReceiveData {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>);
}

impl ReceiveData for WlDataOffer {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
        self.receive(mime_type, fd);
    }
}

impl ReceiveData for ZwpPrimarySelectionOfferV1 {
    fn receive_data(&self, mime_type: String, fd: BorrowedFd<'_>) {
        self.receive(mime_type, fd);
    }
}

#[derive(Clone, Debug)]
/// Wrapper for `WlDataOffer` and `ZwpPrimarySelectionOfferV1`, used to help track mime types.
pub(crate) struct DataOffer<T: ReceiveData> {
    pub inner: T,
    mime_types: Vec<String>,
}

impl<T: ReceiveData> DataOffer<T> {
    pub fn new(offer: T) -> Self {
        Self {
            inner: offer,
            mime_types: Vec::new(),
        }
    }

    pub fn add_mime_type(&mut self, mime_type: String) {
        self.mime_types.push(mime_type)
    }

    pub(crate) fn has_mime_type(&self, mime_type: &str) -> bool {
        self.mime_types.iter().any(|t| t == mime_type)
    }

    fn read_bytes(&self, connection: &Connection, mime_type: &str) -> Option<Vec<u8>> {
        let pipe = Pipe::new().ok()?;
        self.inner.receive_data(mime_type.to_string(), unsafe {
            BorrowedFd::borrow_raw(pipe.write.as_raw_fd())
        });
        let fd = pipe.read;
        drop(pipe.write);

        connection.flush().ok()?;

        match read_fd_with_timeout(fd, PIPE_READ_TIMEOUT) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                log::error!("error reading clipboard pipe: {err:?}");
                None
            }
        }
    }

    fn read_text(&self, connection: &Connection) -> Option<ClipboardItem> {
        let mime_type = self.mime_types.iter().find(|&mime_type| {
            ALLOWED_TEXT_MIME_TYPES
                .iter()
                .any(|&allowed| allowed == mime_type)
        })?;
        let bytes = self.read_bytes(connection, mime_type)?;
        let text_content = match String::from_utf8(bytes) {
            Ok(content) => content,
            Err(e) => {
                log::error!("Failed to convert clipboard content to UTF-8: {}", e);
                return None;
            }
        };

        // Normalize the text to unix line endings, otherwise
        // copying from eg: firefox inserts a lot of blank
        // lines, and that is super annoying.
        let result = text_content.replace("\r\n", "\n");
        Some(ClipboardItem::new_string(result))
    }

    fn read_files(&self, connection: &Connection) -> Option<ClipboardItem> {
        // A combined operation/list takes priority over separate representations.
        let mime = if self.has_mime_type(GNOME_FILES) {
            GNOME_FILES
        } else if self.has_mime_type(URI_LIST) {
            URI_LIST
        } else {
            return None;
        };
        let bytes = self.read_bytes(connection, mime)?;
        let cut = (mime == URI_LIST && self.has_mime_type(KDE_CUT))
            .then(|| self.read_bytes(connection, KDE_CUT))
            .flatten();
        decode_files(mime, &bytes, cut.as_deref())
            .map_err(|error| log::warn!("invalid file clipboard: {error}"))
            .ok()
    }

    fn read_image(&self, connection: &Connection) -> Option<ClipboardItem> {
        for format in ImageFormat::iter() {
            let mime_type = format.mime_type();
            if !self.has_mime_type(mime_type) {
                continue;
            }

            if let Some(bytes) = self.read_bytes(connection, mime_type) {
                let id = hash(&bytes);
                return Some(ClipboardItem {
                    entries: vec![ClipboardEntry::Image(Image { format, bytes, id })],
                });
            }
        }
        None
    }
}

impl Clipboard {
    pub fn new(
        connection: Connection,
        loop_handle: LoopHandle<'static, WaylandClientStatePtr>,
    ) -> Self {
        Self {
            connection,
            loop_handle,
            self_mime: format!("pid/{}", std::process::id()),

            contents: None,
            primary_contents: None,

            cached_read: None,
            current_offer: None,
            cached_primary_read: None,
            current_primary_offer: None,
        }
    }

    pub fn set(&mut self, item: ClipboardItem) {
        self.cached_read = None;
        self.contents = Some(item);
    }

    pub fn set_primary(&mut self, item: ClipboardItem) {
        self.cached_primary_read = None;
        self.primary_contents = Some(item);
    }

    pub fn set_offer(&mut self, data_offer: Option<DataOffer<WlDataOffer>>) {
        self.cached_read = None;
        self.current_offer = data_offer;
    }

    pub fn set_primary_offer(&mut self, data_offer: Option<DataOffer<ZwpPrimarySelectionOfferV1>>) {
        self.cached_primary_read = None;
        self.current_primary_offer = data_offer;
    }

    pub fn self_mime(&self) -> String {
        self.self_mime.clone()
    }

    pub fn send(&self, source: &ClipboardSource, mime_type: &str, fd: OwnedFd) {
        if let Some(bytes) = source.get(mime_type) {
            if let Err(error) = send_bytes(&self.loop_handle, fd, bytes, Duration::from_secs(10)) {
                log::warn!("clipboard transfer failed: {error:#}");
            }
        }
    }

    pub fn read(&mut self) -> Option<ClipboardItem> {
        let offer = self.current_offer.as_ref()?;
        if let Some(cached) = self.cached_read.clone() {
            return Some(cached);
        }

        if offer.has_mime_type(&self.self_mime) {
            return self.contents.clone();
        }

        let item = offer
            .read_files(&self.connection)
            .or_else(|| offer.read_text(&self.connection))
            .or_else(|| offer.read_image(&self.connection))?;

        self.cached_read = Some(item.clone());
        Some(item)
    }

    pub fn read_primary(&mut self) -> Option<ClipboardItem> {
        let offer = self.current_primary_offer.as_ref()?;
        if let Some(cached) = self.cached_primary_read.clone() {
            return Some(cached);
        }

        if offer.has_mime_type(&self.self_mime) {
            return self.primary_contents.clone();
        }

        let item = offer
            .read_files(&self.connection)
            .or_else(|| offer.read_text(&self.connection))
            .or_else(|| offer.read_image(&self.connection))?;

        self.cached_primary_read = Some(item.clone());
        Some(item)
    }
}

fn send_bytes<T: 'static>(
    loop_handle: &LoopHandle<'static, T>,
    fd: OwnedFd,
    bytes: Arc<[u8]>,
    timeout: Duration,
) -> anyhow::Result<()> {
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    anyhow::ensure!(
        flags >= 0
            && unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
        "cannot make clipboard pipe nonblocking"
    );
    if bytes.is_empty() {
        return Ok(());
    }
    let mut written = 0;
    let completed = Rc::new(Cell::new(false));
    let done = completed.clone();
    let token = loop_handle
        .insert_source(
            calloop::generic::Generic::new(
                File::from(fd),
                calloop::Interest::WRITE,
                calloop::Mode::Level,
            ),
            move |_, file, _| {
                let file = unsafe { file.get_mut() };
                let end = (written + 65536).min(bytes.len());
                match file.write(&bytes[written..end]) {
                    Ok(n) if written + n == bytes.len() => {
                        done.set(true);
                        Ok(PostAction::Remove)
                    }
                    Ok(n) if n > 0 => {
                        written += n;
                        Ok(PostAction::Continue)
                    }
                    Err(err)
                        if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) =>
                    {
                        Ok(PostAction::Continue)
                    }
                    _ => {
                        done.set(true);
                        Ok(PostAction::Remove)
                    }
                }
            },
        )
        .map_err(|error| anyhow::anyhow!("cannot register clipboard pipe: {error}"))?;
    let handle = loop_handle.clone();
    if let Err(error) = loop_handle.insert_source(
        calloop::timer::Timer::from_duration(timeout),
        move |_, _, _| {
            if !completed.get() {
                handle.remove(token);
            }
            calloop::timer::TimeoutAction::Drop
        },
    ) {
        loop_handle.remove(token);
        anyhow::bail!("cannot register clipboard timeout: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::time::Instant;

    #[test]
    fn clipboard_transfer_delivers_large_payload_and_closes_fd() {
        let mut event_loop = calloop::EventLoop::<()>::try_new().unwrap();
        let (sender, mut receiver) = UnixStream::pair().unwrap();
        receiver.set_nonblocking(true).unwrap();
        let bytes: Arc<[u8]> = (0..512 * 1024)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>()
            .into();
        let weak = Arc::downgrade(&bytes);
        send_bytes(
            &event_loop.handle(),
            sender.into(),
            bytes.clone(),
            Duration::from_secs(2),
        )
        .unwrap();
        let mut received = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            assert!(Instant::now() < deadline, "clipboard writer did not finish");
            event_loop
                .dispatch(Duration::from_millis(1), &mut ())
                .unwrap();
            let mut buffer = [0; 65536];
            match receiver.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => received.extend_from_slice(&buffer[..n]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(error) => panic!("{error}"),
            }
        }
        assert_eq!(received.as_slice(), &*bytes);
        drop(bytes);
        assert!(
            weak.upgrade().is_none(),
            "completed writer retained clipboard data"
        );
    }

    #[test]
    fn clipboard_transfer_releases_stalled_and_closed_receivers() {
        for disconnected in [false, true] {
            let mut event_loop = calloop::EventLoop::<()>::try_new().unwrap();
            let (sender, receiver) = UnixStream::pair().unwrap();
            let bytes: Arc<[u8]> = vec![42; 2 * 1024 * 1024].into();
            let weak = Arc::downgrade(&bytes);
            send_bytes(
                &event_loop.handle(),
                sender.into(),
                bytes,
                Duration::from_millis(20),
            )
            .unwrap();
            let _receiver = if disconnected {
                drop(receiver);
                None
            } else {
                Some(receiver)
            };
            let deadline = Instant::now() + Duration::from_secs(1);
            while weak.upgrade().is_some() {
                assert!(
                    Instant::now() < deadline,
                    "stalled transfer retained its data"
                );
                event_loop
                    .dispatch(Duration::from_millis(5), &mut ())
                    .unwrap();
            }
        }
    }
}
