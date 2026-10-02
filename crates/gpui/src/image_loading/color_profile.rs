use super::ImageLoadLimits;
use crate::ImageCacheError;
use image::{ColorType, DynamicImage};
use moxcms::{
    ColorProfile, DataColorSpace, Layout, RenderingIntent, TransformExecutor, TransformOptions,
};

pub(super) fn to_srgb(
    image: DynamicImage,
    icc: &[u8],
    limits: ImageLoadLimits,
) -> Result<DynamicImage, ImageCacheError> {
    let profile = match ColorProfile::new_from_slice(icc) {
        Ok(profile) => profile,
        Err(error) => {
            log::warn!("Ignoring invalid PNG ICC profile: {error}");
            return Ok(image);
        }
    };
    let gray = matches!(
        image.color(),
        ColorType::L8 | ColorType::La8 | ColorType::L16 | ColorType::La16
    );
    if profile.color_space
        != if gray {
            DataColorSpace::Gray
        } else {
            DataColorSpace::Rgb
        }
    {
        log::warn!("Ignoring PNG ICC profile with incompatible color space");
        return Ok(image);
    }
    let layout = if gray {
        Layout::GrayAlpha
    } else {
        Layout::Rgba
    };
    let destination = ColorProfile::new_srgb();
    let options = TransformOptions {
        rendering_intent: RenderingIntent::RelativeColorimetric,
        ..Default::default()
    };
    // Keep 16-bit samples until after color conversion. Alpha is always straight
    // coverage and must not pass through a color transfer function.
    if matches!(
        image.color(),
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16
    ) {
        let transform =
            match profile.create_transform_16bit(layout, &destination, Layout::Rgba, options) {
                Ok(transform) => transform,
                Err(error) => {
                    log::warn!("Ignoring unsupported PNG ICC profile: {error}");
                    return Ok(image);
                }
            };
        ImageLoadLimits::check(
            "decoded bytes",
            u64::from(image.width()) * u64::from(image.height()) * 8,
            limits.max_decoded_bytes,
        )?;
        let mut pixels = image.into_rgba16();
        let width = pixels.width() as usize;
        transform_rows(pixels.as_mut(), width, gray, transform.as_ref())?;
        Ok(DynamicImage::ImageRgba16(pixels))
    } else {
        let transform =
            match profile.create_transform_8bit(layout, &destination, Layout::Rgba, options) {
                Ok(transform) => transform,
                Err(error) => {
                    log::warn!("Ignoring unsupported PNG ICC profile: {error}");
                    return Ok(image);
                }
            };
        let mut pixels = image.into_rgba8();
        let width = pixels.width() as usize;
        transform_rows(pixels.as_mut(), width, gray, transform.as_ref())?;
        Ok(DynamicImage::ImageRgba8(pixels))
    }
}

fn transform_rows<T: Copy + Default>(
    pixels: &mut [T],
    width: usize,
    gray: bool,
    transform: &dyn TransformExecutor<T>,
) -> Result<(), ImageCacheError> {
    if width == 0 {
        return Ok(());
    }
    let channels = if gray { 2 } else { 4 };
    let mut source = vec![T::default(); width * channels];
    for row in pixels.chunks_exact_mut(width * 4) {
        if gray {
            for (rgba, ga) in row.chunks_exact(4).zip(source.chunks_exact_mut(2)) {
                ga.copy_from_slice(&[rgba[0], rgba[3]]);
            }
        } else {
            source.copy_from_slice(row);
        }
        transform
            .transform(&source, row)
            .map_err(|error| anyhow::anyhow!("PNG ICC color conversion failed: {error}"))?;
        for (rgba, input) in row.chunks_exact_mut(4).zip(source.chunks_exact(channels)) {
            rgba[3] = input[channels - 1];
        }
    }
    Ok(())
}
