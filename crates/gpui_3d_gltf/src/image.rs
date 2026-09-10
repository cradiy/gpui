use std::{collections::HashMap, io::Cursor, sync::Arc};

use anyhow::{Context, Result, ensure};
use gpui::RenderImage;
use gpui_3d::Material;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};

use crate::{EncodedImage, ImageCache, MaterialDefinition, SceneAsset, SceneDefinition};

/// Admission limits for PNG/JPEG decoding. Output counts are strict; the codec's
/// internal allocation limit is best-effort and is not a process-memory quota.
#[derive(Clone, Copy, Debug)]
pub struct ImageDecodeLimits {
    /// Maximum width and height of each image.
    pub max_dimension: u32,
    /// Maximum pixels per image.
    pub max_pixels: u64,
    /// Aggregate BGRA bytes of unique active image indices in one decode call.
    pub output_bytes: u64,
    /// Per-image admission for encoded bytes, native pixels and BGRA conversion;
    /// also supplied to the codec as its best-effort allocation limit.
    pub working_bytes: u64,
}

impl Default for ImageDecodeLimits {
    fn default() -> Self {
        Self {
            max_dimension: 16_384,
            max_pixels: 16_777_216,
            output_bytes: 256 * 1024 * 1024,
            working_bytes: 256 * 1024 * 1024,
        }
    }
}

impl EncodedImage {
    /// Decodes PNG or JPEG into straight-alpha BGRA8. Embedded color profiles,
    /// gamma, EXIF orientation and animation are not applied. No I/O or GPU work.
    pub fn decode(&self, limits: ImageDecodeLimits) -> Result<Arc<RenderImage>> {
        decode(self, limits, &mut 0)
    }
}

impl MaterialDefinition {
    /// Resolves active PNG/JPEG images with one aggregate output budget.
    /// Custom formats and cache policies can use `resolve_images` instead.
    pub fn decode_images(&self, limits: ImageDecodeLimits) -> Result<Material> {
        let mut output_bytes = 0;
        self.resolve_images(|_, encoded| decode(encoded, limits, &mut output_bytes))
    }

    /// Uses a shared pixel cache while enforcing this material's aggregate budget.
    pub fn decode_images_cached(
        &self,
        cache: &ImageCache,
        limits: ImageDecodeLimits,
    ) -> Result<Material> {
        let mut output_bytes = 0;
        self.resolve_images(|_, encoded| cache.decode_counted(encoded, limits, &mut output_bytes))
    }
}

impl SceneDefinition {
    /// Resolves active PNG/JPEG images with one scene-wide output budget.
    /// An image used by multiple materials is decoded and charged only once.
    pub fn decode_images(&self, limits: ImageDecodeLimits) -> Result<SceneAsset> {
        self.decode_resources(limits)?.resolve()
    }

    /// Decodes active images into transferable CPU resources under one scene-wide
    /// output budget. No GPUI materials or scene graph are constructed.
    pub fn decode_resources(&self, limits: ImageDecodeLimits) -> Result<DecodedScene> {
        let mut output_bytes = 0;
        self.decode_resources_with(|encoded| decode(encoded, limits, &mut output_bytes))
    }

    /// Reuses decoded pixels across calls. Every active image index is charged
    /// once against this call's budget, including indices sharing a cache entry.
    pub fn decode_resources_cached(
        &self,
        cache: &ImageCache,
        limits: ImageDecodeLimits,
    ) -> Result<DecodedScene> {
        let mut output_bytes = 0;
        self.decode_resources_with(|encoded| {
            cache.decode_counted(encoded, limits, &mut output_bytes)
        })
    }

    fn decode_resources_with(
        &self,
        mut decode: impl FnMut(&EncodedImage) -> Result<Arc<RenderImage>>,
    ) -> Result<DecodedScene> {
        let mut images = HashMap::new();
        for material in self.materials() {
            for texture in material.textures() {
                if let std::collections::hash_map::Entry::Vacant(entry) =
                    images.entry(texture.image_index())
                {
                    let image = decode(texture.image()).with_context(|| {
                        format!(
                            "scene {} material {:?} {:?} image {}",
                            self.index(),
                            material.index(),
                            texture.slot(),
                            texture.image_index()
                        )
                    })?;
                    entry.insert(image);
                }
            }
        }
        Ok(DecodedScene {
            definition: self.clone(),
            images: Arc::new(images),
        })
    }
}

/// Shared scene definition and decoded BGRA images, transferable between threads.
/// Encoded inputs remain retained by the definition. Cloning shares both inputs
/// and pixels; resolving constructs graph-local material and subtree values.
#[derive(Clone)]
pub struct DecodedScene {
    definition: SceneDefinition,
    images: Arc<HashMap<usize, Arc<RenderImage>>>,
}

impl DecodedScene {
    pub fn definition(&self) -> &SceneDefinition {
        &self.definition
    }

    /// Returns an active decoded image by its original glTF image index.
    pub fn image(&self, index: usize) -> Option<&Arc<RenderImage>> {
        self.images.get(&index)
    }

    /// Builds a scene asset on the calling thread without decoding again.
    /// Shared pixels are reused; the result includes authored initial deformation.
    pub fn resolve(&self) -> Result<SceneAsset> {
        self.definition.resolve_images(|index, _| {
            self.images
                .get(&index)
                .cloned()
                .context("missing decoded scene image")
        })
    }
}

fn decode(
    encoded: &EncodedImage,
    limits: ImageDecodeLimits,
    used: &mut u64,
) -> Result<Arc<RenderImage>> {
    Ok(decode_image(encoded, limits, used)?.image)
}

#[derive(Clone)]
pub(crate) struct CachedImage {
    pub(crate) image: Arc<RenderImage>,
    info: ImageInfo,
}

impl CachedImage {
    pub(crate) fn admit(&self, limits: ImageDecodeLimits, used: u64) -> Result<u64> {
        self.info.admit(limits, used)
    }

    pub(crate) fn bytes(&self) -> usize {
        self.image.as_bytes(0).unwrap().len()
    }
}

#[derive(Clone, Copy)]
struct ImageInfo {
    width: u32,
    height: u32,
    encoded_bytes: u64,
    native_bytes: u64,
}

impl ImageInfo {
    fn admit(self, limits: ImageDecodeLimits, used: u64) -> Result<u64> {
        let Self {
            width,
            height,
            encoded_bytes,
            native_bytes,
        } = self;
        ensure!(width > 0 && height > 0, "empty image dimensions");
        ensure!(
            width <= limits.max_dimension && height <= limits.max_dimension,
            "image {width}x{height} exceeds dimension limit"
        );
        let pixels = u64::from(width) * u64::from(height);
        ensure!(
            pixels <= limits.max_pixels,
            "image {width}x{height} exceeds pixel limit"
        );
        let output = pixels.checked_mul(4).context("BGRA byte size overflow")?;
        let total = used
            .checked_add(output)
            .context("aggregate image byte size overflow")?;
        ensure!(
            total <= limits.output_bytes,
            "decoded images require {total} bytes, exceeding output byte limit {}",
            limits.output_bytes
        );
        let working = encoded_bytes
            .checked_add(native_bytes)
            .and_then(|bytes| bytes.checked_add(output))
            .context("image working byte size overflow")?;
        ensure!(
            working <= limits.working_bytes,
            "image buffers require {working} bytes, exceeding working byte limit {}",
            limits.working_bytes
        );
        Ok(total)
    }
}

pub(crate) fn decode_image(
    encoded: &EncodedImage,
    limits: ImageDecodeLimits,
    used: &mut u64,
) -> Result<CachedImage> {
    let bytes = encoded.bytes();
    let encoded_bytes = u64::try_from(bytes.len()).context("encoded image size")?;
    ensure!(
        encoded_bytes <= limits.working_bytes,
        "encoded image exceeds working byte limit"
    );
    let format = image::guess_format(bytes).context("image format")?;
    ensure!(
        matches!(format, ImageFormat::Png | ImageFormat::Jpeg),
        "unsupported image format {format:?}; expected PNG or JPEG"
    );
    if let Some(mime) = encoded.mime_type() {
        let expected = match format {
            ImageFormat::Png => "image/png",
            _ => "image/jpeg",
        };
        ensure!(
            mime == expected,
            "image MIME type {mime} conflicts with detected {expected}"
        );
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut codec_limits = image::Limits::default();
    codec_limits.max_image_width = Some(limits.max_dimension);
    codec_limits.max_image_height = Some(limits.max_dimension);
    codec_limits.max_alloc = Some(limits.working_bytes);
    reader.limits(codec_limits);
    let decoder = reader
        .into_decoder()
        .context("image header or decoder limits")?;
    let (width, height) = decoder.dimensions();
    let info = ImageInfo {
        width,
        height,
        encoded_bytes,
        native_bytes: decoder.total_bytes(),
    };
    let total = info.admit(limits, *used)?;
    let mut rgba = DynamicImage::from_decoder(decoder)
        .context("image pixels")?
        .into_rgba8();
    for pixel in rgba.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let result = Arc::new(RenderImage::new(vec![image::Frame::new(rgba)]));
    *used = total;
    Ok(CachedImage {
        image: result,
        info,
    })
}
