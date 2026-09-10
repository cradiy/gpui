use std::{collections::HashMap, ops::Range, sync::Arc};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

/// Admission limits for document parsing and encoded resource preparation.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub document_bytes: usize,
    /// Total unique URI payloads and referenced GLB binary bytes per preparation.
    /// Shared image buffer views do not consume additional bytes.
    pub resource_bytes: usize,
    pub buffers: usize,
    pub images: usize,
    pub accessors: usize,
    pub nodes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            document_bytes: 16 * 1024 * 1024,
            resource_bytes: 256 * 1024 * 1024,
            buffers: 4096,
            images: 4096,
            accessors: 100_000,
            nodes: 100_000,
        }
    }
}

/// Validated JSON/GLB metadata with an optional owned binary chunk. Clones share
/// immutable data. Resources can be prepared repeatedly with independent loaders.
#[derive(Clone, Debug)]
pub struct Document {
    document: Arc<gltf::Document>,
    binary: Option<Arc<[u8]>>,
    limits: Limits,
}

impl Document {
    /// Parses JSON glTF or GLB 2.0 and validates declared byte layouts without I/O.
    pub fn from_slice(bytes: &[u8], limits: Limits) -> Result<Self> {
        admit("document bytes", bytes.len(), limits.document_bytes)?;
        super::validation::container(bytes)?;
        let gltf = gltf::Gltf::from_slice(bytes).context("glTF document")?;
        admit("buffers", gltf.buffers().len(), limits.buffers)?;
        admit("images", gltf.images().len(), limits.images)?;
        admit("accessors", gltf.accessors().len(), limits.accessors)?;
        admit("nodes", gltf.nodes().len(), limits.nodes)?;
        super::validation::layout(&gltf.document)?;
        for buffer in gltf.buffers() {
            if let gltf::buffer::Source::Bin = buffer.source() {
                ensure!(
                    buffer.index() == 0,
                    "buffer {}: only buffer 0 can reference GLB binary data",
                    buffer.index()
                );
                let binary = gltf
                    .blob
                    .as_deref()
                    .context("buffer 0: missing GLB binary chunk")?;
                ensure!(
                    binary.len() >= buffer.length() && binary.len() - buffer.length() <= 3,
                    "buffer 0: GLB binary length {} does not match declared {} bytes with at most three padding bytes",
                    binary.len(),
                    buffer.length()
                );
            }
        }
        Ok(Self {
            document: Arc::new(gltf.document),
            binary: gltf.blob.map(Arc::from),
            limits,
        })
    }

    pub fn gltf(&self) -> &gltf::Document {
        &self.document
    }

    /// Resolves all declared buffers and images. Non-data URIs are passed unchanged
    /// to `load_uri`, including relative paths, schemes, and percent encoding.
    /// Repeated identical URIs are loaded once per call. Base64 data URIs and GLB
    /// chunks are handled internally. No image decoding or scene conversion occurs.
    ///
    /// Callback payload sizes are checked on return; callers must bound their own
    /// I/O allocations. Callback side effects are not rolled back on failure.
    /// The document remains unchanged and can be retried after any failure.
    pub fn prepare(
        &self,
        mut load_uri: impl FnMut(&str) -> Result<Vec<u8>>,
    ) -> Result<PreparedDocument> {
        let mut cache = HashMap::new();
        let mut used = 0;
        let mut buffers = Vec::with_capacity(self.document.buffers().len());
        for buffer in self.document.buffers() {
            admit(
                "declared buffer bytes",
                buffer.length(),
                self.limits.resource_bytes,
            )
            .with_context(|| format!("buffer {}", buffer.index()))?;
            let data = match buffer.source() {
                gltf::buffer::Source::Bin => {
                    let binary = self
                        .binary
                        .as_ref()
                        .context("missing validated GLB binary chunk")?;
                    charge(&mut used, binary.len(), self.limits.resource_bytes)
                        .context("buffer 0")?;
                    binary.clone()
                }
                gltf::buffer::Source::Uri(uri) => resource(
                    uri,
                    &mut cache,
                    &mut used,
                    self.limits.resource_bytes,
                    &mut load_uri,
                )
                .with_context(|| format!("buffer {}", buffer.index()))?,
            };
            ensure!(
                data.len() >= buffer.length(),
                "buffer {}: expected at least {} bytes, received {}",
                buffer.index(),
                buffer.length(),
                data.len()
            );
            buffers.push(Buffer {
                data,
                length: buffer.length(),
            });
        }
        super::validation::sparse(&self.document, |index| {
            buffers.get(index).map(Buffer::bytes)
        })?;
        let mut images = Vec::with_capacity(self.document.images().len());
        for image in self.document.images() {
            let prepared = match image.source() {
                gltf::image::Source::View { view, mime_type } => {
                    let buffer = &buffers[view.buffer().index()];
                    EncodedImage {
                        data: buffer.data.clone(),
                        range: view.offset()..view.offset() + view.length(),
                        mime_type: Some(mime_type.to_owned()),
                    }
                }
                gltf::image::Source::Uri { uri, mime_type } => {
                    let data = resource(
                        uri,
                        &mut cache,
                        &mut used,
                        self.limits.resource_bytes,
                        &mut load_uri,
                    )
                    .with_context(|| format!("image {}", image.index()))?;
                    let embedded_mime = data_uri(uri)?.map(|(mime, _)| mime);
                    if let (Some(declared), Some(embedded)) = (mime_type, embedded_mime) {
                        ensure!(
                            declared == embedded,
                            "image {}: declared MIME type differs from data URI",
                            image.index()
                        );
                    }
                    EncodedImage {
                        range: 0..data.len(),
                        data,
                        mime_type: mime_type.or(embedded_mime).map(str::to_owned),
                    }
                }
            };
            images.push(prepared);
        }
        Ok(PreparedDocument {
            document: self.document.clone(),
            buffers,
            images,
            resource_bytes: used,
        })
    }
}

#[derive(Clone, Debug)]
struct Buffer {
    data: Arc<[u8]>,
    length: usize,
}

impl Buffer {
    fn bytes(&self) -> &[u8] {
        &self.data[..self.length]
    }
}

/// Encoded image data, not decoded pixels. Buffer-view images share buffer storage.
#[derive(Clone, Debug)]
pub struct EncodedImage {
    data: Arc<[u8]>,
    range: Range<usize>,
    mime_type: Option<String>,
}

impl EncodedImage {
    pub fn bytes(&self) -> &[u8] {
        &self.data[self.range.clone()]
    }
    pub fn mime_type(&self) -> Option<&str> {
        self.mime_type.as_deref()
    }
}

/// Owned, shareable resource inputs retaining their original glTF array indices.
/// This is not a rendered scene or a decoded-image cache.
#[derive(Clone, Debug)]
pub struct PreparedDocument {
    document: Arc<gltf::Document>,
    buffers: Vec<Buffer>,
    images: Vec<EncodedImage>,
    resource_bytes: usize,
}

impl PreparedDocument {
    pub fn gltf(&self) -> &gltf::Document {
        &self.document
    }
    /// Returns only the buffer's declared bytes, excluding any GLB padding.
    pub fn buffer(&self, index: usize) -> Option<&[u8]> {
        self.buffers.get(index).map(Buffer::bytes)
    }
    pub fn image(&self, index: usize) -> Option<&EncodedImage> {
        self.images.get(index)
    }
    pub fn resource_bytes(&self) -> usize {
        self.resource_bytes
    }
}

fn resource<'a>(
    uri: &'a str,
    cache: &mut HashMap<&'a str, Arc<[u8]>>,
    used: &mut usize,
    limit: usize,
    load: &mut impl FnMut(&str) -> Result<Vec<u8>>,
) -> Result<Arc<[u8]>> {
    if let Some(data) = cache.get(uri) {
        return Ok(data.clone());
    }
    let bytes = if let Some((_, payload)) = data_uri(uri)? {
        ensure!(
            payload.len().is_multiple_of(4),
            "data URI: invalid base64 length"
        );
        let padding = payload
            .bytes()
            .rev()
            .take_while(|&byte| byte == b'=')
            .count();
        ensure!(padding <= 2, "data URI: invalid base64 padding");
        let decoded = (payload.len() / 4)
            .checked_mul(3)
            .and_then(|n| n.checked_sub(padding))
            .context("data URI: invalid decoded size")?;
        admit("resource bytes", decoded, limit.saturating_sub(*used))?;
        STANDARD
            .decode(payload)
            .context("data URI: invalid base64 payload")?
    } else {
        load(uri).context("URI resolver failed")?
    };
    charge(used, bytes.len(), limit)?;
    let bytes: Arc<[u8]> = bytes.into();
    cache.insert(uri, bytes.clone());
    Ok(bytes)
}

fn data_uri(uri: &str) -> Result<Option<(&str, &str)>> {
    let Some(data) = uri.strip_prefix("data:") else {
        return Ok(None);
    };
    let (header, payload) = data.split_once(',').context("data URI: missing comma")?;
    let Some(mime) = header.strip_suffix(";base64") else {
        bail!("data URI: only base64 encoding is supported")
    };
    ensure!(
        !mime.is_empty() && !mime.contains(';'),
        "data URI: expected an explicit MIME type without parameters"
    );
    Ok(Some((mime, payload)))
}

fn admit(name: &str, actual: usize, limit: usize) -> Result<()> {
    ensure!(actual <= limit, "{name}: {actual} exceeds limit {limit}");
    Ok(())
}

fn charge(used: &mut usize, bytes: usize, limit: usize) -> Result<()> {
    let total = used
        .checked_add(bytes)
        .context("resource byte count overflow")?;
    admit("resource bytes", total, limit)?;
    *used = total;
    Ok(())
}
