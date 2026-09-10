use std::io::Cursor;

use gpui::{DevicePixels, size};
use gpui_3d_gltf::{Document, ImageDecodeLimits, Limits, PreparedDocument, SceneOptions};
use image::{DynamicImage, ImageFormat};
use serde_json::json;

fn encode(image: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, format).unwrap();
    output.into_inner()
}

fn png() -> Vec<u8> {
    encode(
        DynamicImage::ImageRgba8(
            image::RgbaImage::from_raw(2, 1, vec![10, 20, 30, 128, 40, 50, 60, 0]).unwrap(),
        ),
        ImageFormat::Png,
    )
}

fn document(payloads: &[Vec<u8>], mime: Option<&str>) -> PreparedDocument {
    let images: Vec<_> = (0..payloads.len())
        .map(|i| {
            let mut value = json!({"uri":i.to_string()});
            if let Some(mime) = mime {
                value["mimeType"] = json!(mime);
            }
            value
        })
        .collect();
    Document::from_slice(
        &serde_json::to_vec(&json!({"asset":{"version":"2.0"},"images":images})).unwrap(),
        Limits::default(),
    )
    .unwrap()
    .prepare(|uri| Ok(payloads[uri.parse::<usize>()?].clone()))
    .unwrap()
}

#[test]
fn png_preserves_color_hidden_rgb_alpha_and_row_order() {
    let document = document(&[png()], Some("image/png"));
    let decoded = document
        .image(0)
        .unwrap()
        .decode(ImageDecodeLimits::default())
        .unwrap();
    assert_eq!(decoded.size(0), size(DevicePixels(2), DevicePixels(1)));
    assert_eq!(
        decoded.as_bytes(0).unwrap(),
        [30, 20, 10, 128, 60, 50, 40, 0]
    );
    let rgba = image::RgbaImage::from_raw(1, 2, vec![10, 20, 30, 255, 40, 50, 60, 128]).unwrap();
    let document = crate::document(
        &[encode(DynamicImage::ImageRgba8(rgba), ImageFormat::Png)],
        None,
    );
    assert_eq!(
        document
            .image(0)
            .unwrap()
            .decode(ImageDecodeLimits::default())
            .unwrap()
            .as_bytes(0)
            .unwrap(),
        [30, 20, 10, 255, 60, 50, 40, 128]
    );
}

#[test]
fn grayscale_and_sixteen_bit_png_expand_into_bgra8() {
    let inputs = [
        (
            DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_pixel(
                1,
                1,
                image::LumaA([64, 128]),
            )),
            [64, 64, 64, 128],
        ),
        (
            DynamicImage::ImageRgba16(image::ImageBuffer::from_pixel(
                1,
                1,
                image::Rgba([0x1212, 0x3434, 0x5656, 0xabab]),
            )),
            [0x56, 0x34, 0x12, 0xab],
        ),
    ];
    for (input, expected) in inputs {
        let document = document(&[encode(input, ImageFormat::Png)], None);
        let decoded = document
            .image(0)
            .unwrap()
            .decode(ImageDecodeLimits::default())
            .unwrap();
        assert_eq!(decoded.as_bytes(0).unwrap(), expected);
    }
}

#[test]
fn jpeg_decodes_rgb_without_alpha_premultiplication() {
    let input =
        DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb([180, 60, 20])));
    let document = document(&[encode(input, ImageFormat::Jpeg)], Some("image/jpeg"));
    let decoded = document
        .image(0)
        .unwrap()
        .decode(ImageDecodeLimits::default())
        .unwrap();
    for pixel in decoded.as_bytes(0).unwrap().chunks_exact(4) {
        for (&actual, expected) in pixel.iter().zip([20_i16, 60, 180, 255]) {
            assert!((i16::from(actual) - expected).abs() <= 3);
        }
        assert_eq!(pixel[3], 255);
    }
}

#[test]
fn invalid_formats_mime_and_corruption_return_errors() {
    let mut truncated = png();
    truncated.truncate(truncated.len() / 2);
    for (bytes, mime) in [
        (vec![], None),
        (truncated, None),
        (png(), Some("image/jpeg")),
        (
            encode(DynamicImage::new_rgba8(1, 1), ImageFormat::Gif),
            None,
        ),
    ] {
        let document = document(&[bytes], mime);
        assert!(
            document
                .image(0)
                .unwrap()
                .decode(ImageDecodeLimits::default())
                .is_err()
        );
    }
}

#[test]
fn dimension_pixel_output_and_working_limits_are_checked_and_retryable() {
    let document = document(&[png()], None);
    let image = document.image(0).unwrap();
    let defaults = ImageDecodeLimits::default();
    for limits in [
        ImageDecodeLimits {
            max_dimension: 1,
            ..defaults
        },
        ImageDecodeLimits {
            max_pixels: 1,
            ..defaults
        },
        ImageDecodeLimits {
            output_bytes: 7,
            ..defaults
        },
        ImageDecodeLimits {
            working_bytes: 1,
            ..defaults
        },
        ImageDecodeLimits {
            working_bytes: image.bytes().len() as u64 + 15,
            ..defaults
        },
    ] {
        assert!(image.decode(limits).is_err());
    }
    let decoded = image
        .decode(ImageDecodeLimits {
            max_dimension: 2,
            max_pixels: 2,
            output_bytes: 8,
            working_bytes: image.bytes().len() as u64 + 16,
        })
        .unwrap();
    assert_eq!(decoded.as_bytes(0).unwrap().len(), 8);
}

#[test]
fn image_cache_matches_contents_and_mime_without_retaining_source_buffers() {
    use gpui_3d_gltf::{ImageCache, ImageCacheLimits};
    use std::sync::Arc;

    let cache = ImageCache::new(ImageCacheLimits {
        bytes: 32,
        entries: 4,
    });
    let bytes: Arc<[u8]> = png().into();
    let weak = Arc::downgrade(&bytes);
    let metadata = Document::from_slice(
        br#"{"asset":{"version":"2.0"},"images":[{"uri":"image","mimeType":"image/png"}]}"#,
        Limits::default(),
    )
    .unwrap();
    let source = futures::executor::block_on(
        metadata.prepare_shared_async(|_| std::future::ready(Ok(bytes.clone()))),
    )
    .unwrap();
    let first = cache
        .decode(source.image(0).unwrap(), ImageDecodeLimits::default())
        .unwrap();
    drop(source);
    drop(bytes);
    assert!(weak.upgrade().is_none());
    let same = document(&[png()], Some("image/png"));
    let second = cache
        .decode(same.image(0).unwrap(), ImageDecodeLimits::default())
        .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let invalid_mime = document(&[png()], Some("image/jpeg"));
    assert!(
        cache
            .decode(invalid_mime.image(0).unwrap(), ImageDecodeLimits::default())
            .is_err()
    );
    let changed = document(
        &[encode(DynamicImage::new_rgba8(2, 1), ImageFormat::Png)],
        Some("image/png"),
    );
    let different = cache
        .decode(changed.image(0).unwrap(), ImageDecodeLimits::default())
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &different));
    assert_eq!(different.as_bytes(0).unwrap(), [0; 8]);
    assert_eq!(cache.len(), 2);
    assert_eq!(cache.cached_bytes(), 16);
    assert!(cache.invalidate(same.image(0).unwrap()));
    assert!(!cache.invalidate(same.image(0).unwrap()));
    let decoded = cache
        .decode(same.image(0).unwrap(), ImageDecodeLimits::default())
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &decoded));
    cache.clear();
    assert!(cache.is_empty());
    assert_eq!(cache.cached_bytes(), 0);
    assert_eq!(first.as_bytes(0).unwrap(), [30, 20, 10, 128, 60, 50, 40, 0]);
}

#[test]
fn cached_pixels_obey_current_decode_limits_and_lru_retention() {
    use gpui_3d_gltf::{ImageCache, ImageCacheLimits};
    use std::sync::Arc;

    let first_source = document(&[png()], None);
    let second_source = document(
        &[encode(DynamicImage::new_rgba8(2, 1), ImageFormat::Png)],
        None,
    );
    let first_image = first_source.image(0).unwrap();
    let second_image = second_source.image(0).unwrap();
    let cache = ImageCache::new(ImageCacheLimits {
        bytes: 16,
        entries: 2,
    });
    let defaults = ImageDecodeLimits::default();
    let first = cache.decode(first_image, defaults).unwrap();
    for limits in [
        ImageDecodeLimits {
            max_dimension: 1,
            ..defaults
        },
        ImageDecodeLimits {
            max_pixels: 1,
            ..defaults
        },
        ImageDecodeLimits {
            output_bytes: 7,
            ..defaults
        },
        ImageDecodeLimits {
            working_bytes: first_image.bytes().len() as u64 + 15,
            ..defaults
        },
    ] {
        assert!(cache.decode(first_image, limits).is_err());
        assert_eq!(cache.len(), 1);
    }
    let hit = cache
        .decode(
            first_image,
            ImageDecodeLimits {
                max_dimension: 2,
                max_pixels: 2,
                output_bytes: 8,
                working_bytes: first_image.bytes().len() as u64 + 16,
            },
        )
        .unwrap();
    assert!(Arc::ptr_eq(&first, &hit));
    let second = cache.decode(second_image, defaults).unwrap();
    let weak = Arc::downgrade(&second);
    drop(second);
    cache.decode(first_image, defaults).unwrap();
    cache.set_limits(ImageCacheLimits {
        bytes: 8,
        entries: 2,
    });
    assert!(weak.upgrade().is_none());
    assert_eq!(cache.cached_bytes(), 8);
    assert!(Arc::ptr_eq(
        &first,
        &cache.decode(first_image, defaults).unwrap()
    ));
    cache.set_limits(ImageCacheLimits {
        bytes: 7,
        entries: 2,
    });
    assert!(cache.is_empty());
    let oversized = cache.decode(first_image, defaults).unwrap();
    assert!(cache.is_empty());
    assert!(!Arc::ptr_eq(&first, &oversized));
    cache.set_limits(ImageCacheLimits {
        bytes: 16,
        entries: 0,
    });
    cache.decode(first_image, defaults).unwrap();
    assert!(cache.is_empty());
}

#[test]
fn embedded_glb_scene_decoding_charges_unique_images_across_materials() {
    let positions: Vec<_> = [[0_f32, 0., 0.], [1., 0., 0.], [0., 1., 0.]]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let uv: Vec<_> = [[0_f32, 0.], [1., 0.], [0., 1.]]
        .into_iter()
        .flatten()
        .flat_map(f32::to_le_bytes)
        .collect();
    let image = png();
    let mut binary = positions;
    binary.extend(uv);
    binary.extend(&image);
    let mut source = json!({"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],"nodes":[{"mesh":0}],
        "buffers":[{"byteLength":binary.len()}],
        "bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":24},
            {"buffer":0,"byteOffset":60,"byteLength":image.len()}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
            {"bufferView":1,"componentType":5126,"count":3,"type":"VEC2"}],
        "images":[{"bufferView":2,"mimeType":"image/png"}],"textures":[{"source":0}],
        "materials":[{"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}},
            {"pbrMetallicRoughness":{"baseColorTexture":{"index":0}},"occlusionTexture":{"index":0}}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":0},
            {"attributes":{"POSITION":0,"TEXCOORD_0":1},"material":1}]}]});
    let prepare = |source: &serde_json::Value| {
        let mut json = serde_json::to_vec(source).unwrap();
        json.resize(json.len().next_multiple_of(4), b' ');
        let mut bin = binary.clone();
        bin.resize(bin.len().next_multiple_of(4), 0);
        let mut glb: Vec<_> = [
            0x46546c67_u32,
            2,
            (28 + json.len() + bin.len()) as u32,
            json.len() as u32,
            0x4e4f534a,
        ]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
        glb.extend(json);
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(0x004e4942_u32.to_le_bytes());
        glb.extend(bin);
        Document::from_slice(&glb, Limits::default())
            .unwrap()
            .prepare(|_| panic!("embedded resources"))
            .unwrap()
    };
    let limits = ImageDecodeLimits {
        output_bytes: 8,
        ..Default::default()
    };
    let document = prepare(&source);
    let definition = document.scene(None, SceneOptions::default()).unwrap();
    assert_eq!(
        definition.decode_images(limits).unwrap().primitives().len(),
        2
    );
    let worker_definition = definition.clone();
    let cache = gpui_3d_gltf::ImageCache::default();
    let warmed = definition.decode_resources_cached(&cache, limits).unwrap();
    let worker_cache = cache.clone();
    let decoded = std::thread::spawn(move || {
        worker_definition.decode_resources_cached(&worker_cache, limits)
    })
    .join()
    .unwrap()
    .unwrap();
    assert_eq!(decoded.definition().index(), definition.index());
    assert!(std::sync::Arc::ptr_eq(
        warmed.image(0).unwrap(),
        decoded.image(0).unwrap()
    ));
    assert_eq!(
        decoded.image(0).unwrap().as_bytes(0).unwrap(),
        [30, 20, 10, 128, 60, 50, 40, 0]
    );
    assert!(decoded.image(1).is_none());
    let retained = decoded.clone();
    assert!(std::sync::Arc::ptr_eq(
        decoded.image(0).unwrap(),
        retained.image(0).unwrap()
    ));
    drop(decoded);
    assert_eq!(retained.resolve().unwrap().primitives().len(), 2);
    assert!(
        document
            .material(Some(1))
            .unwrap()
            .decode_images(limits)
            .is_ok()
    );
    source["images"]
        .as_array_mut()
        .unwrap()
        .push(json!({"bufferView":2,"mimeType":"image/png"}));
    source["textures"]
        .as_array_mut()
        .unwrap()
        .push(json!({"source":1}));
    source["materials"][1]["occlusionTexture"]["index"] = json!(1);
    let document = prepare(&source);
    let definition = document.scene(None, SceneOptions::default()).unwrap();
    assert!(definition.decode_resources_cached(&cache, limits).is_err());
    assert!(
        document
            .material(Some(1))
            .unwrap()
            .decode_images_cached(&cache, limits)
            .is_err()
    );
    let shared_indices = definition
        .decode_resources_cached(
            &cache,
            ImageDecodeLimits {
                output_bytes: 16,
                ..limits
            },
        )
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(
        shared_indices.image(0).unwrap(),
        shared_indices.image(1).unwrap()
    ));
    assert_eq!(cache.len(), 1);
    assert_eq!(cache.cached_bytes(), 8);
    let error = format!("{:#}", definition.decode_images(limits).err().unwrap());
    assert!(
        error.contains("image 1") && error.contains("output byte limit"),
        "{error}"
    );
    assert!(
        definition
            .decode_images(ImageDecodeLimits {
                output_bytes: 16,
                ..limits
            })
            .is_ok()
    );
    assert!(
        document
            .material(Some(1))
            .unwrap()
            .decode_images(limits)
            .is_err()
    );
}
