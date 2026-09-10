use std::{io::Cursor, sync::Arc};

use gpui::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, ImageSource, RenderImage,
    Scene3dFrame, TileId, point, size,
};
use gpui_3d::{
    AlphaMode, Material, Mesh, Object, ResolvedTexture, Scene, TextureAddressMode,
    TextureColorSpace, TextureFilter, TextureMipFilter, TextureSlot, TextureSource, TextureState,
};
use gpui_3d_gltf::{
    Document, EncodedImage, GeometryOptions, Limits, MaterialDefinition, PreparedDocument,
};
use serde_json::{Value, json};

fn prepare(mut value: Value) -> PreparedDocument {
    value["asset"] = json!({"version":"2.0"});
    Document::from_slice(&serde_json::to_vec(&value).unwrap(), Limits::default())
        .unwrap()
        .prepare(|_| Ok(vec![1, 2, 3]))
        .unwrap()
}

fn textured(material: Value) -> Value {
    json!({"materials":[material],"textures":[{"source":0}],"images":[{"uri":"map.png","mimeType":"image/png"}]})
}

fn pixels() -> Arc<RenderImage> {
    Arc::new(RenderImage::new(vec![image::Frame::new(
        image::RgbaImage::from_pixel(2, 2, image::Rgba([30, 20, 10, 128])),
    )]))
}

fn frame(material: Material) -> Scene3dFrame {
    Scene::new()
        .object(Object::new(Mesh::plane(), material))
        .prepare(1., None, |request| {
            Ok(TextureState::Ready(match request.source {
                TextureSource::Solid => ResolvedTexture::None,
                TextureSource::Image(ImageSource::Render(image)) => {
                    assert_eq!(image.size(0), size(DevicePixels(2), DevicePixels(2)));
                    ResolvedTexture::Image(AtlasTile {
                        texture_id: AtlasTextureId {
                            index: 0,
                            kind: AtlasTextureKind::Polychrome,
                        },
                        tile_id: TileId(0),
                        padding: 0,
                        bounds: Bounds::new(
                            point(DevicePixels(0), DevicePixels(0)),
                            size(DevicePixels(2), DevicePixels(2)),
                        ),
                    })
                }
                _ => panic!("expected decoded image"),
            }))
        })
        .unwrap()
        .frame()
        .clone()
}

fn resolve(definition: &MaterialDefinition) -> Scene3dFrame {
    frame(definition.resolve_images(|_, _| Ok(pixels())).unwrap())
}

fn error(document: &PreparedDocument) -> String {
    format!("{:#}", document.material(Some(0)).err().unwrap())
}

#[test]
fn material_factors_and_default_alpha_faces_reach_render_preparation() {
    let document = prepare(json!({"materials":[{
        "pbrMetallicRoughness":{"baseColorFactor":[0.003,0.25,0.75,0.4],"metallicFactor":0.7,"roughnessFactor":0.2},
        "emissiveFactor":[0.1,0.2,0.3],"alphaMode":"MASK","alphaCutoff":0.,"doubleSided":true
    }]}));
    let default = document.material(None).unwrap();
    let default_frame = frame(
        default
            .resolve_images(|_, _| panic!("default has no images"))
            .unwrap(),
    );
    let object = &default_frame.objects[0];
    assert_eq!(object.alpha_mode, AlphaMode::Opaque);
    assert!(!object.double_sided);
    assert_eq!(object.pbr.unwrap().metallic, 1.);
    assert_eq!(object.pbr.unwrap().roughness, 1.);
    let definition = document.material(Some(0)).unwrap();
    let prepared = resolve(&definition);
    let object = &prepared.objects[0];
    assert_eq!(object.alpha_cutoff, 0.);
    assert_eq!(object.alpha_mode, AlphaMode::Mask);
    assert!(object.double_sided);
    let factors = object.pbr.unwrap();
    assert_eq!(
        (factors.metallic, factors.roughness, factors.emissive),
        (0.7, 0.2, [0.1, 0.2, 0.3])
    );
    let decode_srgb = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    for (actual, expected) in [object.color.r, object.color.g, object.color.b]
        .into_iter()
        .zip([0.003, 0.25, 0.75])
    {
        assert!((decode_srgb(actual) - expected).abs() < 2e-6);
    }
    assert_eq!(object.color.a, 0.4);
    assert!(document.material(Some(1)).is_err());
}

#[test]
fn five_map_materials_share_decoding_and_preserve_per_slot_sampling() {
    let transform = json!({"KHR_texture_transform":{"offset":[0.25,0.5],"scale":[2.,3.],"rotation":std::f32::consts::FRAC_PI_2,"texCoord":1}});
    let info = json!({"index":0,"extensions":transform});
    let mut normal = info.clone();
    normal["scale"] = json!(0.6);
    let mut occlusion = info.clone();
    occlusion["strength"] = json!(0.25);
    let mut value = textured(json!({
        "pbrMetallicRoughness":{"baseColorFactor":[0.5,0.2,0.1,0.75],"baseColorTexture":info,"metallicRoughnessTexture":info},
        "normalTexture":normal,"occlusionTexture":occlusion,"emissiveTexture":info,"emissiveFactor":[0.2,0.3,0.4],"alphaMode":"BLEND"
    }));
    value["extensionsUsed"] = json!(["KHR_texture_transform"]);
    value["samplers"] = json!([{"wrapS":33648,"wrapT":33071,"minFilter":9985,"magFilter":9729}]);
    value["textures"][0]["sampler"] = json!(0);
    let document = prepare(value);
    let definition = document.material(Some(0)).unwrap();
    assert_eq!(definition.tex_coord_set(), Some(1));
    assert!(definition.requires_tangents());
    assert_eq!(definition.textures().len(), 5);
    for binding in definition.textures() {
        assert_eq!((binding.texture_index(), binding.image_index()), (0, 0));
        assert_eq!(binding.image().bytes(), [1, 2, 3]);
        assert_eq!(
            binding.color_space(),
            match binding.slot() {
                TextureSlot::BaseColor | TextureSlot::Emissive => TextureColorSpace::Srgb,
                _ => TextureColorSpace::Linear,
            }
        );
        let sampling = binding.sampling();
        assert_eq!(sampling.address_u, TextureAddressMode::Mirror);
        assert_eq!(sampling.address_v, TextureAddressMode::Clamp);
        assert_eq!(sampling.filter, TextureFilter::Linear);
        assert_eq!(sampling.mip_filter, TextureMipFilter::Nearest);
        let mapped = sampling.transform.transform([0.5, 0.25]).unwrap();
        assert!((mapped[0] + 0.5).abs() < 1e-6 && (mapped[1] - 1.5).abs() < 1e-6);
    }
    let mut calls = 0;
    let image = pixels();
    let material = definition
        .resolve_images(|index, encoded| {
            calls += 1;
            assert_eq!(index, 0);
            assert_eq!(encoded.mime_type(), Some("image/png"));
            Ok(image.clone())
        })
        .unwrap();
    assert_eq!(calls, 1);
    drop(document);
    let prepared = frame(material);
    let object = &prepared.objects[0];
    assert_eq!(object.alpha_mode, AlphaMode::Blend);
    assert!(!object.double_sided);
    assert_eq!(object.color.a, 0.75);
    assert_eq!(object.normal_scale, 0.6);
    assert_eq!(object.occlusion_strength, 0.25);
    assert_eq!(object.image_color_space, TextureColorSpace::Srgb);
    assert_eq!(object.sampling, definition.textures()[0].sampling());
    assert!(
        object.metallic_roughness_texture.is_some()
            && object.emissive_texture.is_some()
            && object.normal_texture.is_some()
            && object.occlusion_texture.is_some()
    );
}

#[test]
fn sampler_modes_preserve_independent_minification_and_magnification() {
    for (min, filter, mip) in [
        (9728, TextureFilter::Nearest, TextureMipFilter::None),
        (9729, TextureFilter::Linear, TextureMipFilter::None),
        (9984, TextureFilter::Nearest, TextureMipFilter::Nearest),
        (9985, TextureFilter::Linear, TextureMipFilter::Nearest),
        (9986, TextureFilter::Nearest, TextureMipFilter::Linear),
        (9987, TextureFilter::Linear, TextureMipFilter::Linear),
    ] {
        let mut value = textured(json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}));
        value["samplers"] = json!([{"minFilter":min}]);
        value["textures"][0]["sampler"] = json!(0);
        let document = prepare(value.clone());
        let definition = document.material(Some(0)).unwrap();
        let sampling = definition.textures()[0].sampling();
        assert_eq!((sampling.filter, sampling.mip_filter), (filter, mip));
        assert_eq!(sampling.address_u, TextureAddressMode::Repeat);
        assert_eq!(sampling.magnification_filter(), filter);
        for (mag, expected) in [
            (9728, TextureFilter::Nearest),
            (9729, TextureFilter::Linear),
        ] {
            value["samplers"][0]["magFilter"] = json!(mag);
            let definition = prepare(value.clone()).material(Some(0)).unwrap();
            let sampling = definition.textures()[0].sampling();
            assert_eq!((sampling.filter, sampling.mip_filter), (filter, mip));
            assert_eq!(sampling.magnification_filter(), expected);
        }
    }
    let document = prepare(textured(
        json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}),
    ));
    let definition = document.material(Some(0)).unwrap();
    let sampling = definition.textures()[0].sampling();
    assert_eq!(
        (sampling.filter, sampling.mip_filter),
        (TextureFilter::Linear, TextureMipFilter::Linear)
    );
}

#[test]
fn inactive_maps_do_not_require_uvs_tangents_or_decoding() {
    for unlit in [false, true] {
        let mut material = json!({"normalTexture":{"index":0,"scale":0.,"texCoord":8},
            "occlusionTexture":{"index":0,"strength":0.,"texCoord":9},"emissiveTexture":{"index":0,"texCoord":10}});
        if unlit {
            material["extensions"] = json!({"KHR_materials_unlit":{}});
            material["normalTexture"]["scale"] = json!(1.);
            material["occlusionTexture"]["strength"] = json!(1.);
            material["emissiveFactor"] = json!([1., 1., 1.]);
            material["pbrMetallicRoughness"] =
                json!({"metallicRoughnessTexture":{"index":0,"texCoord":11}});
        }
        let document = prepare(textured(material));
        let definition = document.material(Some(0)).unwrap();
        assert!(definition.textures().is_empty());
        assert_eq!(definition.tex_coord_set(), None);
        assert!(!definition.requires_tangents());
        let material = definition
            .resolve_images(|_, _| panic!("inactive image decoded"))
            .unwrap();
        assert_eq!(frame(material).objects[0].unlit, unlit);
    }
}

#[test]
fn malformed_factors_transforms_and_uv_conflicts_are_recoverable() {
    for material in [
        json!({"pbrMetallicRoughness":{"baseColorFactor":[-1.,0.,0.,1.]}}),
        json!({"pbrMetallicRoughness":{"metallicFactor":2.}}),
        json!({"pbrMetallicRoughness":{"roughnessFactor":-1.}}),
        json!({"emissiveFactor":[2.,0.,0.]}),
        json!({"alphaMode":"MASK","alphaCutoff":-0.1}),
        json!({"normalTexture":{"index":0,"scale":-1.}}),
        json!({"occlusionTexture":{"index":0,"strength":2.}}),
        json!({"normalTexture":{"index":0,"extensions":{"KHR_texture_transform":{"scale":[1.]}}}}),
    ] {
        let document = prepare(textured(material));
        assert!(error(&document).contains("material Some(0)"));
        assert!(document.material(None).is_ok());
    }
    let value = textured(
        json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0}},"occlusionTexture":{"index":0,"texCoord":1}}),
    );
    assert!(error(&prepare(value)).contains("multiple active UV sets"));
}

#[test]
fn image_resolution_failures_include_binding_identity_and_allow_retry() {
    let document = prepare(textured(
        json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0}}}),
    ));
    let definition = document.material(Some(0)).unwrap();
    let error = definition
        .resolve_images(|_, _| anyhow::bail!("decoder failed"))
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("BaseColor texture 0 image 0: decoder failed"));
    let empty = Arc::new(RenderImage::new(Vec::<image::Frame>::new()));
    assert!(definition.resolve_images(|_, _| Ok(empty.clone())).is_err());
    drop(document);
    let output = definition.resolve_images(|_, _| Ok(pixels())).unwrap();
    assert!(matches!(
        frame(output).objects[0].texture,
        ResolvedTexture::Image(_)
    ));
}

#[test]
fn encoded_png_pixels_survive_color_and_data_bindings_without_transfer_conversion() {
    let rgba = image::RgbaImage::from_pixel(2, 2, image::Rgba([10, 20, 30, 128]));
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let mut value = textured(
        json!({"pbrMetallicRoughness":{"baseColorTexture":{"index":0},"metallicRoughnessTexture":{"index":0}}}),
    );
    value["asset"] = json!({"version":"2.0"});
    let document = Document::from_slice(&serde_json::to_vec(&value).unwrap(), Limits::default())
        .unwrap()
        .prepare(|_| Ok(png.get_ref().clone()))
        .unwrap();
    let definition = document.material(Some(0)).unwrap();
    let mut calls = 0;
    let material = definition
        .resolve_images(|_, encoded: &EncodedImage| {
            calls += 1;
            let mut decoded = image::load_from_memory(encoded.bytes())?.into_rgba8();
            for pixel in decoded.pixels_mut() {
                pixel.0.swap(0, 2);
            }
            let image = Arc::new(RenderImage::new(vec![image::Frame::new(decoded)]));
            assert_eq!(&image.as_bytes(0).unwrap()[..4], [30, 20, 10, 128]);
            Ok(image)
        })
        .unwrap();
    assert_eq!(calls, 1);
    assert!(
        frame(material).objects[0]
            .metallic_roughness_texture
            .is_some()
    );
}

#[test]
fn material_geometry_checks_require_matching_uvs_and_active_tangents() {
    let document = prepare(textured(json!({"normalTexture":{"index":0,"texCoord":1}})));
    let definition = document.material(Some(0)).unwrap();
    let mut value = json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":96,"uri":"mesh.bin"}],
        "bufferViews":[{"buffer":0,"byteLength":96,"byteStride":32}],
        "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},
            {"bufferView":0,"byteOffset":12,"componentType":5126,"count":3,"type":"VEC3"},
            {"bufferView":0,"byteOffset":24,"componentType":5126,"count":3,"type":"VEC2"}],
        "meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"TEXCOORD_1":2}}]}]});
    let bytes: Vec<_> = [
        [0_f32, 0., 0., 0., 0., 1., 0., 0.],
        [1., 0., 0., 0., 0., 1., 1., 0.],
        [0., 1., 0., 0., 0., 1., 0., 1.],
    ]
    .into_iter()
    .flatten()
    .flat_map(f32::to_le_bytes)
    .collect();
    let geometry_document = |value: &Value| {
        Document::from_slice(&serde_json::to_vec(value).unwrap(), Limits::default())
            .unwrap()
            .prepare(|_| Ok(bytes.clone()))
            .unwrap()
    };
    let geometry = geometry_document(&value)
        .geometry(
            0,
            0,
            GeometryOptions {
                tex_coord_set: 1,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        format!("{:#}", definition.validate_geometry(&geometry).unwrap_err())
            .contains("requires tangents")
    );
    let geometry = geometry_document(&value)
        .geometry(
            0,
            0,
            GeometryOptions {
                tex_coord_set: 1,
                generate_tangents: true,
                ..Default::default()
            },
        )
        .unwrap();
    definition.validate_geometry(&geometry).unwrap();
    value["meshes"][0]["primitives"][0]["attributes"] = json!({"POSITION":0,"NORMAL":1});
    let geometry = geometry_document(&value)
        .geometry(0, 0, GeometryOptions::default())
        .unwrap();
    assert!(
        format!("{:#}", definition.validate_geometry(&geometry).unwrap_err()).contains("UV set")
    );
}
