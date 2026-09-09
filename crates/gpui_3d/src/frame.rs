use crate::{Scene, Texture, TextureSlot};
use anyhow::{Result, ensure};
use gpui::{MeshDraw3d, MeshTexture3d, Scene3dFrame, UiTexture3d};

impl Scene {
    pub(crate) fn prepare_frame(
        &self,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
        mut resolve: impl FnMut(usize, TextureSlot, &Texture) -> Result<Option<MeshTexture3d>>,
    ) -> Result<Scene3dFrame> {
        let view_projection = self.camera.view_projection(aspect)?;
        let view = self.camera.view_matrix()?;
        ensure!(
            self.color_output.is_valid(),
            "scene exposure must be finite and between -16 and 16 stops"
        );
        let light = self.light;
        ensure!(
            light
                .direction
                .iter()
                .chain([
                    &light.intensity,
                    &light.ambient,
                    &light.color.r,
                    &light.color.g,
                    &light.color.b,
                ])
                .all(|value| value.is_finite()),
            "scene light parameters must be finite"
        );
        ensure!(
            self.objects.len() < u32::MAX as usize,
            "too many objects for 32-bit output IDs"
        );
        let mut objects = Vec::with_capacity(self.objects.len());
        for (index, object) in self.objects.iter().enumerate() {
            ensure!(
                object.material.pbr.is_none_or(|pbr| pbr.is_valid()),
                "object {index} has invalid PBR parameters"
            );
            ensure!(
                object.material.normal_scale.is_finite() && object.material.normal_scale >= 0.,
                "object {index} has invalid normal scale"
            );
            ensure!(
                !object
                    .material
                    .pbr_textures()
                    .any(|(slot, _)| slot == TextureSlot::Normal)
                    || object.mesh.tangents().is_some(),
                "object {index}: normal maps require mesh tangents"
            );
            let transform = object.transform;
            ensure!(
                object.world.is_some()
                    || (transform
                        .position
                        .iter()
                        .chain(&transform.rotation)
                        .chain(&transform.scale)
                        .all(|v| v.is_finite())
                        && transform.scale.iter().all(|v| v.abs() >= 0.0001)),
                "object {index} has an invalid transform"
            );
            let (model, normal) = object.matrices();
            let sort_depth = if object.material.alpha_mode == crate::AlphaMode::Blend {
                let bounds = object.mesh.bounds();
                let center: [f64; 4] = std::array::from_fn(|i| {
                    if i == 3 {
                        1.
                    } else {
                        (f64::from(bounds.min()[i]) + f64::from(bounds.max()[i])) * 0.5
                    }
                });
                let world: [f64; 4] = std::array::from_fn(|r| {
                    (0..4).map(|c| f64::from(model[c][r]) * center[c]).sum()
                });
                -(0..4)
                    .map(|c| f64::from(view[c][2]) * world[c])
                    .sum::<f64>()
            } else {
                0.
            };
            ensure!(
                sort_depth.is_finite(),
                "object {index} has invalid sort depth"
            );
            let color = object.material.color;
            ensure!(
                model
                    .iter()
                    .flatten()
                    .chain(normal.iter().flatten())
                    .chain([
                        &color.r,
                        &color.g,
                        &color.b,
                        &color.a,
                        &object.material.alpha_cutoff
                    ])
                    .all(|value| value.is_finite()),
                "object {index} has non-finite render parameters"
            );
            let mut metallic_roughness_texture = None;
            let mut emissive_texture = None;
            let mut normal_texture = None;
            let mut ready = true;
            for (slot, map) in object.material.pbr_textures() {
                match resolve(index, slot, &Texture::Image(map.image.clone()))? {
                    Some(MeshTexture3d::Image(tile)) => {
                        let resolved = Some(gpui::MaterialTexture3d {
                            tile,
                            sampling: map.sampling,
                        });
                        match slot {
                            TextureSlot::MetallicRoughness => metallic_roughness_texture = resolved,
                            TextureSlot::Emissive => emissive_texture = resolved,
                            TextureSlot::Normal => normal_texture = resolved,
                            TextureSlot::BaseColor => unreachable!(),
                        }
                    }
                    None => ready = false,
                    _ => anyhow::bail!("object {index}: material maps require atlas images"),
                }
            }
            if !ready {
                continue;
            }
            let Some(texture) = resolve(index, TextureSlot::BaseColor, &object.material.texture)?
            else {
                continue;
            };
            objects.push(MeshDraw3d {
                output_id: index as u32 + 1,
                mesh: object.mesh.0.clone(),
                model,
                normal,
                color,
                texture,
                sampling: object.material.sampling,
                image_color_space: object.material.image_color_space,
                pbr: object.material.pbr,
                metallic_roughness_texture,
                emissive_texture,
                normal_texture,
                normal_scale: object.material.normal_scale,
                alpha_cutoff: object.material.alpha_cutoff,
                alpha_mode: object.material.alpha_mode,
                sort_depth,
                unlit: object.material.unlit,
            });
        }
        Ok(Scene3dFrame {
            world_to_view: view,
            ui_texture,
            view_projection,
            camera_position: self.camera.eye,
            orthographic_view_direction: match self.camera.projection {
                crate::Projection::Perspective { .. } => None,
                crate::Projection::Orthographic { .. } => {
                    Some([view[0][2], view[1][2], view[2][2]])
                }
            },
            light_direction: light.direction,
            light: [
                light.color.r,
                light.color.g,
                light.color.b,
                light.intensity.max(0.),
            ],
            ambient: light.ambient.max(0.),
            color_output: self.color_output,
            objects: objects.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AffineTransform, Camera, Material, Mesh, Node, Object, SceneGraph};
    use gpui::rgb;

    #[test]
    fn blend_sort_depth_uses_camera_forward_and_transformed_bounds() {
        let camera = Camera {
            eye: [4., 0., 0.],
            target: [0.; 3],
            ..Default::default()
        };
        let material = Material::color(rgb(0xffffff)).alpha_mode(crate::AlphaMode::Blend);
        let frame = Scene::new()
            .camera(camera)
            .object(Object::new(Mesh::plane(), material.clone()).position([2., 100., 0.]))
            .object(Object::new(Mesh::plane(), material).position([0., 0., 0.]))
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        assert!((frame.objects[0].sort_depth - 2.).abs() < 1e-6);
        assert!((frame.objects[1].sort_depth - 4.).abs() < 1e-6);
        assert_eq!(frame.objects[0].output_id, 1);
        assert_eq!(frame.objects[1].output_id, 2);
    }

    #[test]
    fn normal_maps_require_tangents_only_when_active() {
        use crate::{MaterialTexture, PbrMaterial};
        let plane = Mesh::plane();
        let mesh = Mesh::new(plane.vertices().to_vec(), plane.indices().to_vec());
        let material = Material::color(rgb(0xffffff))
            .pbr(PbrMaterial::default())
            .normal_texture(MaterialTexture::new("normal.png"));
        let scene = |material| Scene::new().object(Object::new(mesh.clone(), material));
        assert!(
            scene(material.clone())
                .prepare_frame(1., None, |_, _, _| unreachable!())
                .is_err()
        );
        for inactive in [
            material.clone().normal_scale(0.),
            material.clone().unlit(true),
        ] {
            let frame = scene(inactive)
                .prepare_frame(1., None, |_, slot, _| {
                    assert_eq!(slot, TextureSlot::BaseColor);
                    Ok(Some(MeshTexture3d::None))
                })
                .unwrap();
            assert!(frame.objects[0].normal_texture.is_none());
        }
        for scale in [-1., f32::NAN, f32::INFINITY] {
            assert!(
                scene(material.clone().normal_scale(scale))
                    .prepare_frame(1., None, |_, _, _| unreachable!())
                    .is_err()
            );
        }
    }

    #[test]
    fn material_map_readiness_preserves_ids_and_resolves_only_active_inputs() {
        use crate::{MaterialTexture, PbrMaterial, TextureSampling, UvTransform};
        let sampling = TextureSampling {
            transform: UvTransform::from_rows([[2., 0., 0.25], [0., 3., -0.5]]).unwrap(),
            ..Default::default()
        };
        let material = Material::color(rgb(0xffffff))
            .pbr(PbrMaterial::default())
            .metallic_roughness_texture(MaterialTexture::new("surface.png").sampling(sampling))
            .emissive_texture(MaterialTexture::new("emission.png"));
        let tile = gpui::AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 1,
                kind: gpui::AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(2),
            padding: 0,
            bounds: gpui::Bounds::new(
                gpui::point(gpui::DevicePixels(4), gpui::DevicePixels(8)),
                gpui::size(gpui::DevicePixels(16), gpui::DevicePixels(16)),
            ),
        };
        let scene = Scene::new()
            .object(Object::new(Mesh::plane(), material.clone()))
            .object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))));
        for ready in [false, true] {
            let mut base_objects = Vec::new();
            let frame = scene
                .prepare_frame(1., None, |index, slot, _| {
                    Ok(match slot {
                        TextureSlot::BaseColor => {
                            base_objects.push(index);
                            Some(MeshTexture3d::None)
                        }
                        TextureSlot::Emissive if !ready => None,
                        _ => Some(MeshTexture3d::Image(tile)),
                    })
                })
                .unwrap();
            let ids: Vec<_> = frame
                .objects
                .iter()
                .map(|object| object.output_id)
                .collect();
            if ready {
                assert_eq!(ids, [1, 2]);
                assert_eq!(base_objects, [0, 1]);
                assert_eq!(
                    frame.objects[0]
                        .metallic_roughness_texture
                        .unwrap()
                        .sampling,
                    sampling
                );
                assert_eq!(frame.objects[0].emissive_texture.unwrap().tile, tile);
            } else {
                assert_eq!(ids, [2]);
                assert_eq!(base_objects, [1]);
            }
        }
        let mut diffuse = material.clone();
        diffuse.pbr = None;
        for inactive in [material.unlit(true), diffuse] {
            let frame = Scene::new()
                .object(Object::new(Mesh::plane(), inactive))
                .prepare_frame(1., None, |_, slot, _| {
                    assert_eq!(slot, TextureSlot::BaseColor);
                    Ok(Some(MeshTexture3d::None))
                })
                .unwrap();
            assert!(frame.objects[0].metallic_roughness_texture.is_none());
            assert!(frame.objects[0].emissive_texture.is_none());
        }
        assert!(
            scene
                .prepare_frame(1., None, |_, slot, _| {
                    if slot == TextureSlot::Emissive {
                        anyhow::bail!("decode failed");
                    }
                    Ok(Some(MeshTexture3d::Image(tile)))
                })
                .is_err()
        );
    }

    #[test]
    fn prepared_frames_share_geometry_and_preserve_evaluated_world_transforms() {
        let mut graph = SceneGraph::new();
        let parent = graph
            .insert(
                None,
                Node::new().transform(AffineTransform::from_translation([3., 2., -1.]).unwrap()),
            )
            .unwrap();
        let child = graph
            .insert(
                Some(parent),
                Node::new()
                    .id("mesh")
                    .mesh(Mesh::cube(), Material::color(rgb(0x80a0c0)))
                    .transform(AffineTransform::from_translation([1., 0., 0.]).unwrap()),
            )
            .unwrap();
        let evaluated = graph.evaluate().unwrap();
        let scene = evaluated.scene(Camera::default());
        let a = scene
            .prepare_frame(1.5, None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        let b = scene
            .prepare_frame(0.5, None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        assert_eq!(
            a.objects[0].model,
            evaluated.node(child).unwrap().world.matrix()
        );
        assert_eq!(
            a.objects[0].normal,
            evaluated.node(child).unwrap().world.normal_matrix()
        );
        assert!(std::sync::Arc::ptr_eq(
            &a.objects[0].mesh,
            &b.objects[0].mesh
        ));
        assert_ne!(a.view_projection, b.view_projection);
        assert_eq!(a.objects[0].output_id, b.objects[0].output_id);
        graph.remove_subtree(parent).unwrap();
        assert_eq!(a.objects[0].model[3][0], 4.);
    }

    #[test]
    fn omitted_resources_do_not_renumber_ids_and_invalid_input_returns_errors() {
        let scene = Scene::new()
            .object(Object::new(Mesh::plane(), Material::image("pending.png")))
            .object(Object::new(Mesh::cube(), Material::color(rgb(0x808080))));
        let frame = scene
            .prepare_frame(1., None, |_, _, texture| {
                Ok(match texture {
                    Texture::Image(_) => None,
                    _ => Some(MeshTexture3d::None),
                })
            })
            .unwrap();
        assert_eq!(frame.objects.len(), 1);
        assert_eq!(frame.objects[0].output_id, 2);
        assert!(
            scene
                .prepare_frame(0., None, |_, _, _| unreachable!())
                .is_err()
        );
        let invalid = Scene::new()
            .object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))).scale([0., 1., 1.]));
        assert!(
            invalid
                .prepare_frame(1., None, |_, _, _| unreachable!())
                .is_err()
        );
        assert!(
            scene
                .prepare_frame(1., None, |_, _, _| anyhow::bail!("image unavailable"))
                .is_err()
        );
        for exposure in [f32::NAN, f32::INFINITY, -17., 17.] {
            assert!(
                scene
                    .clone()
                    .color_output(crate::ColorOutput {
                        exposure,
                        ..Default::default()
                    })
                    .prepare_frame(1., None, |_, _, _| unreachable!())
                    .is_err()
            );
        }
    }

    #[test]
    fn pbr_frames_keep_view_geometry_and_reject_invalid_materials() {
        use crate::{PbrMaterial, Projection};
        let material = PbrMaterial {
            metallic: 0.7,
            roughness: 0.2,
            emissive: [0.1, 2., 0.3],
        };
        let scene = Scene::new().object(Object::new(
            Mesh::plane(),
            Material::color(rgb(0x806040)).pbr(material),
        ));
        let camera = Camera::orbit(0.6, 0.3, 5.);
        let prepare = |camera| {
            scene
                .clone()
                .camera(camera)
                .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
                .unwrap()
        };
        let perspective = prepare(camera);
        assert_eq!(perspective.camera_position, camera.eye);
        assert!(perspective.orthographic_view_direction.is_none());
        assert_eq!(perspective.objects[0].pbr, Some(material));
        let ortho = Camera {
            projection: Projection::Orthographic { vertical_size: 3. },
            ..camera
        };
        let direction = prepare(ortho).orthographic_view_direction.unwrap();
        let displacement = [1., 2., -3.];
        let translated = Camera {
            eye: std::array::from_fn(|i| ortho.eye[i] + displacement[i]),
            target: std::array::from_fn(|i| ortho.target[i] + displacement[i]),
            ..ortho
        };
        let translated_direction = prepare(translated).orthographic_view_direction.unwrap();
        for (a, b) in direction.into_iter().zip(translated_direction) {
            assert!((a - b).abs() < 1e-6);
        }
        for invalid in [
            PbrMaterial {
                metallic: -0.1,
                ..material
            },
            PbrMaterial {
                metallic: 1.1,
                ..material
            },
            PbrMaterial {
                roughness: f32::NAN,
                ..material
            },
            PbrMaterial {
                roughness: 1.1,
                ..material
            },
            PbrMaterial {
                emissive: [-1., 0., 0.],
                ..material
            },
            PbrMaterial {
                emissive: [f32::INFINITY; 3],
                ..material
            },
        ] {
            let scene = Scene::new().object(Object::new(
                Mesh::plane(),
                Material::color(rgb(0xffffff)).pbr(invalid),
            ));
            assert!(
                scene
                    .prepare_frame(1., None, |_, _, _| unreachable!())
                    .is_err()
            );
        }
    }
}
