#[cfg(test)]
use crate::Texture;
use crate::{Scene, TextureSlot};
use anyhow::{Result, ensure};
use gpui::{MeshDraw3d, MeshTexture3d, Scene3dFrame, UiTexture3d};

impl Scene {
    #[cfg(test)]
    pub(crate) fn prepare_frame(
        &self,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
        resolve: impl FnMut(usize, TextureSlot, &Texture) -> Result<Option<MeshTexture3d>>,
    ) -> Result<Scene3dFrame> {
        self.bind_frame(&self.plan_frame(aspect, ui_texture)?, resolve)
    }

    pub(super) fn plan_frame(
        &self,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
    ) -> Result<Scene3dFrame> {
        let view_projection = self.camera.view_projection(aspect)?;
        let view = self.camera.view_matrix()?;
        ensure!(
            self.color_output.is_valid(),
            "scene exposure must be finite and between -16 and 16 stops"
        );
        let light = self.light;
        if let Some(lights) = &self.lights {
            ensure!(
                lights.len() <= crate::MAX_PUNCTUAL_LIGHTS,
                "too many direct lights; maximum is {}",
                crate::MAX_PUNCTUAL_LIGHTS
            );
            for (index, light) in lights.iter().enumerate() {
                ensure!(
                    light.is_valid(),
                    "direct light {index} has invalid parameters"
                );
            }
        }
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
        let directional_shadow = self
            .directional_shadow
            .map(|shadow| {
                let direction = if let Some(lights) = &self.lights {
                    let source = lights.get(shadow.light_index as usize).ok_or_else(|| {
                        anyhow::anyhow!("directional shadow light index is out of range")
                    })?;
                    ensure!(
                        source.kind == gpui::LightKind3d::Directional,
                        "shadow source must be directional"
                    );
                    source.direction
                } else {
                    ensure!(
                        shadow.light_index == 0,
                        "single light shadow index must be zero"
                    );
                    light.direction
                };
                shadow.prepare(direction)
            })
            .transpose()?;
        ensure!(
            self.objects.len() < u32::MAX as usize,
            "too many objects for 32-bit output IDs"
        );
        let mut objects = Vec::with_capacity(self.objects.len());
        for (index, object) in self.objects.iter().enumerate() {
            ensure!(
                !matches!(object.material.texture, crate::Texture::Image(_))
                    || object.material.sampling.is_valid(),
                "object {index} has invalid image sampling"
            );
            ensure!(
                !matches!(object.material.texture, crate::Texture::Image(_))
                    || object.mesh.uv_at(object.material.uv_set, 0).is_some(),
                "object {index}: missing base-color UV set {}",
                object.material.uv_set
            );
            for (slot, texture) in object.material.lighting_textures() {
                ensure!(
                    object.mesh.uv_at(texture.uv_set, 0).is_some(),
                    "object {index}: missing {slot:?} UV set {}",
                    texture.uv_set
                );
                ensure!(
                    slot != TextureSlot::Normal
                        || object.mesh.tangent_uv_set() == Some(texture.uv_set),
                    "object {index}: normal maps require mesh tangents for UV set {}",
                    texture.uv_set
                );
                ensure!(
                    texture.sampling.is_valid(),
                    "object {index} has invalid {slot:?} sampling"
                );
            }
            ensure!(
                object.material.pbr.is_none_or(|pbr| pbr.is_valid()),
                "object {index} has invalid PBR parameters"
            );
            ensure!(
                object.material.normal_scale.is_finite() && object.material.normal_scale >= 0.,
                "object {index} has invalid normal scale"
            );
            ensure!(
                object.material.alpha_cutoff >= 0.,
                "object {index} has a negative alpha cutoff"
            );
            ensure!(
                object.material.occlusion_strength.is_finite()
                    && (0. ..=1.).contains(&object.material.occlusion_strength),
                "object {index} has invalid occlusion strength"
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
                let bounds = object.render_bounds.unwrap_or_else(|| object.mesh.bounds());
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
            let bounds = object.render_bounds.unwrap_or_else(|| object.mesh.bounds());
            let intersects = |projection| {
                gpui::Mesh3d::bounds_intersect_clip_volume(
                    [bounds.min(), bounds.max()],
                    model,
                    projection,
                )
            };
            let camera_visible = intersects(view_projection);
            let shadow_visible = object.cast_shadows
                && object.material.alpha_mode != crate::AlphaMode::Blend
                && directional_shadow.is_some_and(|shadow| intersects(shadow.view_projection));
            if !camera_visible && !shadow_visible {
                continue;
            }
            objects.push(MeshDraw3d {
                gpu_geometry: object.gpu_geometry.clone(),
                render_bounds: object
                    .render_bounds
                    .map(|bounds| [bounds.min(), bounds.max()]),
                cast_shadows: object.cast_shadows,
                receive_shadows: object.receive_shadows,
                output_id: index as u32 + 1,
                mesh: object.mesh.0.clone(),
                model,
                normal,
                color,
                texture: MeshTexture3d::None,
                sampling: object.material.sampling,
                uv_set: object.material.uv_set,
                image_color_space: object.material.image_color_space,
                pbr: object.material.pbr,
                metallic_roughness_texture: None,
                emissive_texture: None,
                normal_texture: None,
                normal_scale: object.material.normal_scale,
                occlusion_texture: None,
                occlusion_strength: object.material.occlusion_strength,
                alpha_cutoff: object.material.alpha_cutoff,
                double_sided: object.material.double_sided,
                alpha_mode: object.material.alpha_mode,
                sort_depth,
                unlit: object.material.unlit,
            });
        }
        Ok(Scene3dFrame {
            pick_capture: None,
            depth_background: if self.camera.near == 0. {
                gpui::DepthBackground3d::NegativeOne
            } else {
                gpui::DepthBackground3d::Zero
            },
            viewport_quality: Default::default(),
            specular_environment: self
                .specular_environment
                .as_ref()
                .map(|environment| environment.0.clone()),
            background: self
                .background
                .as_ref()
                .map(|background| background.prepare(self.camera, aspect))
                .transpose()?,
            directional_shadow,
            diffuse_environment: self.diffuse_environment.map(|environment| environment.0),
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
            lights: self.lights.clone(),
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
    fn independent_coordinate_sets_reach_all_material_slots() {
        use crate::{MaterialTexture, PbrMaterial};
        let mut mesh = Mesh::plane();
        for set in [2, 7, 11, 19, u32::MAX] {
            mesh = mesh
                .with_uv_set(set, mesh.vertices().iter().map(|v| v.uv).collect())
                .unwrap();
        }
        mesh = mesh
            .with_tangents_for_uv_set(19, mesh.tangents().unwrap().to_vec())
            .unwrap();
        let material = Material::image("base.png")
            .image_uv_set(2)
            .pbr(PbrMaterial::default())
            .metallic_roughness_texture(MaterialTexture::new("surface.png").uv_set(7))
            .emissive_texture(MaterialTexture::new("emission.png").uv_set(11))
            .normal_texture(MaterialTexture::new("normal.png").uv_set(19))
            .occlusion_texture(MaterialTexture::new("occlusion.png").uv_set(u32::MAX));
        let tile = gpui::AtlasTile {
            texture_id: gpui::AtlasTextureId {
                index: 0,
                kind: gpui::AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(0),
            padding: 0,
            bounds: gpui::Bounds::new(
                gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
                gpui::size(gpui::DevicePixels(1), gpui::DevicePixels(1)),
            ),
        };
        let scene = Scene::new().object(Object::new(mesh.clone(), material.clone()));
        let frame = scene
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::Image(tile))))
            .unwrap();
        assert_eq!(frame.objects[0].texture_uv_sets(), [2, 7, 11, 19, u32::MAX]);
        let missing =
            Scene::new().object(Object::new(mesh.clone(), material.clone().image_uv_set(3)));
        assert!(
            missing
                .prepare_frame(1., None, |_, _, _| panic!("missing coordinates"))
                .is_err()
        );
        let mismatch = Scene::new().object(Object::new(
            mesh.clone(),
            material.normal_texture(MaterialTexture::new("normal.png").uv_set(7)),
        ));
        assert!(
            mismatch
                .prepare_frame(1., None, |_, _, _| panic!("mismatched normal basis"))
                .is_err()
        );
        let ui = Scene::new().object(Object::new(
            mesh,
            Material::ui()
                .image_uv_set(3)
                .normal_texture(MaterialTexture::new("unused.png").uv_set(3)),
        ));
        let frame = ui
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::Subtree)))
            .unwrap();
        assert_eq!(frame.objects[0].texture_uv_sets(), [0; 5]);
    }

    #[test]
    fn frame_culling_skips_resources_but_keeps_shadow_casters_and_original_ids() {
        let material = Material::color(rgb(0xffffff));
        let camera = Camera {
            eye: [0., 0., 4.],
            target: [0.; 3],
            near: 0.1,
            far: 10.,
            projection: crate::Projection::Orthographic { vertical_size: 2. },
            ..Default::default()
        };
        let scene = Scene::new()
            .camera(camera)
            .lights([crate::PunctualLight::directional([0., 0., 1.])])
            .directional_shadow(Some(crate::DirectionalShadow::new(
                [3., 0., 0.],
                [1., 2., 2.],
            )))
            .object(
                Object::new(Mesh::cube(), material.clone())
                    .position([100., 0., 0.])
                    .id("distant"),
            )
            .object(Object::new(Mesh::cube(), material.clone()))
            .object(Object::new(Mesh::cube(), material.clone()).position([3., 0., 0.]))
            .object(
                Object::new(Mesh::cube(), material.clone())
                    .position([3., 0., 0.])
                    .cast_shadows(false),
            )
            .object(
                Object::new(Mesh::cube(), material.alpha_mode(crate::AlphaMode::Blend))
                    .position([3., 0., 0.]),
            );
        let mut resolved = Vec::new();
        let frame = scene
            .prepare_frame(1., None, |index, _, _| {
                resolved.push(index);
                Ok(Some(MeshTexture3d::None))
            })
            .unwrap();
        assert_eq!(resolved, vec![1, 2]);
        assert_eq!(
            frame
                .objects
                .iter()
                .map(|o| o.output_id)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(
            scene
                .raycast(crate::Ray::new([100., 0., 4.], [0., 0., -1.]).unwrap())
                .unwrap()
                .object_id,
            Some("distant".into())
        );
        let scene = scene.directional_shadow(None);
        let frame = scene
            .prepare_frame(1., None, |index, _, _| {
                assert_eq!(index, 1);
                Ok(Some(MeshTexture3d::None))
            })
            .unwrap();
        assert_eq!(frame.objects.len(), 1);
        assert_eq!(frame.objects[0].output_id, 2);
        let moved = scene.camera(Camera {
            eye: [3., 0., 4.],
            target: [3., 0., 0.],
            ..camera
        });
        let frame = moved
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        assert_eq!(
            frame
                .objects
                .iter()
                .map(|o| o.output_id)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );
    }

    #[test]
    fn shadow_state_survives_graph_evaluation_and_is_independent_of_view_camera() {
        use crate::{DirectionalShadow, PunctualLight};
        let mut graph = SceneGraph::new();
        graph
            .insert(
                None,
                Node::new()
                    .mesh(Mesh::cube(), Material::color(rgb(0xffffff)))
                    .cast_shadows(false)
                    .receive_shadows(false),
            )
            .unwrap();
        let scene = graph
            .evaluate()
            .unwrap()
            .scene(Camera::default())
            .lights([
                PunctualLight::point([0., 1., 0.]),
                PunctualLight::directional([1., 2., 3.]),
            ])
            .directional_shadow(Some(DirectionalShadow {
                light_index: 1,
                ..DirectionalShadow::new([0.; 3], [4.; 3])
            }));
        let prepare = |scene: &Scene| {
            scene
                .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
                .unwrap()
        };
        let original = prepare(&scene);
        let moved = prepare(&scene.camera(Camera::orbit(1., 0.4, 5.)));
        assert!(original.shadow_is_valid());
        assert_eq!(
            original.directional_shadow.unwrap().view_projection,
            moved.directional_shadow.unwrap().view_projection
        );
        assert_ne!(original.view_projection, moved.view_projection);
        assert!(!original.objects[0].cast_shadows && !original.objects[0].receive_shadows);
        assert_eq!(original.objects[0].output_id, moved.objects[0].output_id);
    }

    #[test]
    fn invalid_shadow_sources_and_volumes_fail_before_texture_resolution() {
        use crate::{DirectionalShadow, PunctualLight};
        let settings = DirectionalShadow::new([0.; 3], [4.; 3]);
        let scene = Scene::new().object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))));
        let mut invalid = vec![
            scene.clone().lights([]).directional_shadow(Some(settings)),
            scene
                .clone()
                .lights([PunctualLight::point([0., 0., 1.])])
                .directional_shadow(Some(settings)),
        ];
        for settings in [
            DirectionalShadow {
                light_index: 1,
                ..settings
            },
            DirectionalShadow {
                center: [f32::NAN, 0., 0.],
                ..settings
            },
            DirectionalShadow {
                half_extent: [0., 1., 1.],
                ..settings
            },
            DirectionalShadow {
                resolution: 16384,
                ..settings
            },
            DirectionalShadow {
                resolution: 1000,
                ..settings
            },
            DirectionalShadow {
                depth_bias: -0.1,
                ..settings
            },
            DirectionalShadow {
                normal_bias: f32::INFINITY,
                ..settings
            },
            DirectionalShadow {
                softness: 5.,
                ..settings
            },
        ] {
            invalid.push(scene.clone().directional_shadow(Some(settings)));
        }
        for scene in invalid {
            assert!(
                scene
                    .prepare_frame(1., None, |_, _, _| unreachable!())
                    .is_err()
            );
        }
    }

    #[test]
    fn blend_sort_depth_uses_camera_forward_and_transformed_bounds() {
        let camera = Camera {
            eye: [4., 0., 0.],
            target: [0.; 3],
            projection: crate::Projection::Orthographic {
                vertical_size: 220.,
            },
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
                assert_eq!(base_objects, [0, 1]);
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
    fn occlusion_resources_follow_lit_state_and_preserve_object_identity() {
        use crate::{MaterialTexture, PbrMaterial, TextureSampling, UvTransform};
        let sampling = TextureSampling {
            transform: UvTransform::from_rows([[2., 0., 0.25], [0., 3., -0.5]]).unwrap(),
            ..Default::default()
        };
        let material = Material::color(rgb(0xffffff))
            .occlusion_texture(MaterialTexture::new("occlusion.png").sampling(sampling));
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
        for material in [
            material.clone(),
            material.clone().pbr(PbrMaterial::default()),
        ] {
            for (strength, unlit, ready) in [
                (1., false, false),
                (0.4, false, true),
                (0., false, false),
                (1., true, false),
            ] {
                let active = strength > 0. && !unlit;
                let scene = Scene::new()
                    .object(Object::new(
                        Mesh::plane(),
                        material.clone().occlusion_strength(strength).unlit(unlit),
                    ))
                    .object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))));
                let mut requests = Vec::new();
                let frame = scene
                    .prepare_frame(1., None, |index, slot, _| {
                        requests.push((index, slot));
                        Ok(if slot == TextureSlot::Occlusion {
                            ready.then_some(MeshTexture3d::Image(tile))
                        } else {
                            Some(MeshTexture3d::None)
                        })
                    })
                    .unwrap();
                assert_eq!(requests.contains(&(0, TextureSlot::Occlusion)), active);
                let ids: Vec<_> = frame
                    .objects
                    .iter()
                    .map(|object| object.output_id)
                    .collect();
                assert_eq!(
                    ids,
                    if active && !ready {
                        vec![2]
                    } else {
                        vec![1, 2]
                    }
                );
                if active && ready {
                    let map = frame.objects[0].occlusion_texture.unwrap();
                    assert_eq!(map.tile, tile);
                    assert_eq!(map.sampling, sampling);
                    assert_eq!(frame.objects[0].occlusion_strength, strength);
                }
            }
        }
        for strength in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
            let scene = Scene::new().object(Object::new(
                Mesh::plane(),
                material.clone().occlusion_strength(strength),
            ));
            assert!(
                scene
                    .prepare_frame(1., None, |_, _, _| unreachable!(
                        "invalid strength must fail before resource resolution"
                    ))
                    .is_err()
            );
        }
    }

    #[test]
    fn explicit_lights_replace_direct_sources_without_changing_ambient_or_objects() {
        use crate::{Light, PunctualLight};
        let scene = Scene::new()
            .light(Light {
                ambient: 0.17,
                ..Default::default()
            })
            .object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))));
        let prepare = |scene: &Scene| {
            scene
                .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
                .unwrap()
        };
        let sources = [
            PunctualLight::point([1., 2., 3.]).range(Some(5.)),
            PunctualLight::spot([0., 0., 2.], [0., 0., -1.]),
        ];
        let multiple = prepare(&scene.clone().lights(sources));
        assert_eq!(multiple.ambient, 0.17);
        assert_eq!(multiple.objects[0].output_id, 1);
        let lights = multiple.lights.as_ref().unwrap();
        assert_eq!(lights.len(), 2);
        assert_eq!(lights[0].position, [1., 2., 3.]);
        assert_eq!(lights[0].range, Some(5.));
        assert_eq!(lights[1].direction, [0., 0., -1.]);
        assert!(
            prepare(&scene.clone().lights([]))
                .lights
                .unwrap()
                .is_empty()
        );
        assert_eq!(prepare(&scene.clone().lights([])).ambient, 0.17);
        assert!(
            prepare(&scene.clone().lights(sources).light(Light::default()))
                .lights
                .is_none()
        );
        assert!(prepare(&scene).lights.is_none());
    }

    #[test]
    fn light_limits_and_invalid_sources_fail_before_resource_resolution() {
        use crate::{MAX_PUNCTUAL_LIGHTS, PunctualLight};
        let light = PunctualLight::point([0., 0., 1.]);
        let scene = |lights: Vec<_>| {
            Scene::new()
                .lights(lights)
                .object(Object::new(Mesh::plane(), Material::color(rgb(0xffffff))))
        };
        assert!(
            scene(vec![light; MAX_PUNCTUAL_LIGHTS])
                .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
                .is_ok()
        );
        let invalid = [
            PunctualLight::directional([0.; 3]),
            PunctualLight::point([f32::NAN, 0., 0.]),
            light.intensity(-1.),
            light.intensity(f32::INFINITY),
            light.range(Some(0.)),
            light.range(Some(f32::INFINITY)),
            light.minimum_distance(0.),
            PunctualLight::spot([0.; 3], [0.; 3]),
            PunctualLight::spot([0.; 3], [0., 0., -1.]).cone_angles(0.5, 0.5),
            PunctualLight::spot([0.; 3], [0., 0., -1.]).cone_angles(0., 2.),
            PunctualLight::spot([0.; 3], [0., 0., -1.]).cone_angles(0., 1e-8),
        ];
        for lights in
            invalid
                .into_iter()
                .map(|light| vec![light])
                .chain([vec![light; MAX_PUNCTUAL_LIGHTS + 1]])
        {
            assert!(
                scene(lights)
                    .prepare_frame(1., None, |_, _, _| unreachable!(
                        "invalid lights must fail before resource resolution"
                    ))
                    .is_err()
            );
        }
    }

    #[test]
    fn morphed_frames_and_rays_use_the_same_mesh_before_hierarchy_transforms() {
        use crate::{MorphTarget, MorphTargets, Ray};
        let source = Mesh::plane();
        let morphs = MorphTargets::new(
            source.clone(),
            [MorphTarget {
                positions: Some(vec![[0.4, 0., 0.25]; 4].into()),
                normals: Some(vec![[1., 0., 0.]; 4].into()),
                ..Default::default()
            }],
        )
        .unwrap();
        let mesh = morphs.evaluate(&[1.]).unwrap();
        let mut graph = SceneGraph::new();
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let parent = graph
            .insert(
                None,
                Node::new().transform(
                    AffineTransform::from_trs([2., 3., 1.], [0., 0., k, k], [2., 1., 0.5]).unwrap(),
                ),
            )
            .unwrap();
        let child = graph
            .insert(
                Some(parent),
                Node::new()
                    .id("morph")
                    .mesh(mesh.clone(), Material::color(rgb(0xffffff)))
                    .transform(AffineTransform::from_translation([0.5, 0., 0.]).unwrap()),
            )
            .unwrap();
        let evaluated = graph.evaluate().unwrap();
        let camera = Camera::default()
            .frame_bounds(evaluated.bounds().unwrap(), 1., 1.2)
            .unwrap();
        let scene = evaluated.scene(camera);
        let frame = scene
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        let object = &frame.objects[0];
        assert!(std::sync::Arc::ptr_eq(&object.mesh, &mesh.0));
        assert!(std::ptr::eq(object.mesh.indices(), source.indices()));
        let hit = scene
            .raycast(Ray::new([2., 4.8, 5.], [0., 0., -1.]).unwrap())
            .unwrap();
        assert_eq!(hit.node, Some(child));
        for (a, b) in hit.position.into_iter().zip([2., 4.8, 1.125]) {
            assert!((a - b).abs() < 1e-5);
        }
        for (a, b) in hit
            .normal
            .into_iter()
            .zip([0., 1. / 17_f32.sqrt(), 4. / 17_f32.sqrt()])
        {
            assert!((a - b).abs() < 1e-5);
        }
        graph.set_mesh(child, source).unwrap();
        assert_eq!(frame.objects[0].output_id, 1);
        assert_eq!(
            frame.objects[0].mesh.vertices()[0].position,
            mesh.vertices()[0].position
        );
        assert!(
            (scene
                .raycast(Ray::new([2., 4.8, 5.], [0., 0., -1.]).unwrap())
                .unwrap()
                .position[2]
                - 1.125)
                .abs()
                < 1e-5
        );
    }

    #[test]
    fn skinned_hierarchy_poses_share_deformed_vertices_with_render_frames_and_rays() {
        use crate::{Ray, Skin, SkinInfluence};
        let source = Mesh::plane();
        let skin = Skin::new(
            [AffineTransform::IDENTITY; 2],
            source.vertices().iter().map(|v| {
                [SkinInfluence {
                    joint: usize::from(v.position[1] > 0.),
                    weight: 1.,
                }]
            }),
        )
        .unwrap();
        let mut graph = SceneGraph::new();
        let root = graph
            .insert(
                None,
                Node::new().transform(
                    AffineTransform::from_trs([2., 3., 0.], [0., 0., 0., 1.], [2., 1., 1.])
                        .unwrap(),
                ),
            )
            .unwrap();
        let lower = graph.insert(Some(root), Node::new()).unwrap();
        let upper = graph
            .insert(
                Some(root),
                Node::new().transform(AffineTransform::from_translation([0., 0., 1.]).unwrap()),
            )
            .unwrap();
        let body = graph
            .insert(
                Some(root),
                Node::new()
                    .mesh(source.clone(), Material::color(rgb(0xffffff)))
                    .transform(AffineTransform::from_translation([0., -1., 0.]).unwrap()),
            )
            .unwrap();
        let pose = graph.evaluate().unwrap();
        let mesh = skin
            .evaluate_world(
                &source,
                pose.node(body).unwrap().world,
                &[
                    pose.node(lower).unwrap().world,
                    pose.node(upper).unwrap().world,
                ],
            )
            .unwrap();
        graph.set_mesh(body, mesh.clone()).unwrap();
        let evaluated = graph.evaluate().unwrap();
        let bounds = evaluated.bounds().unwrap();
        assert_eq!(bounds.min(), [1., 2.5, 0.]);
        assert_eq!(bounds.max(), [3., 3.5, 1.]);
        let scene = evaluated.scene(Camera::default().frame_bounds(bounds, 1., 1.2).unwrap());
        let frame = scene
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        assert!(std::sync::Arc::ptr_eq(&frame.objects[0].mesh, &mesh.0));
        assert!(std::ptr::eq(
            frame.objects[0].mesh.indices(),
            source.indices()
        ));
        let ray = Ray::new([2., 3., 4.], [0., 0., -1.]).unwrap();
        let hit = scene.raycast(ray).unwrap();
        assert_eq!(hit.node, Some(body));
        assert!((hit.position[2] - 0.5).abs() < 1e-5);
        assert!((hit.uv[0] - 0.5).abs() < 1e-5 && (hit.uv[1] - 0.5).abs() < 1e-5);
        graph.set_mesh(body, source).unwrap();
        assert!((scene.raycast(ray).unwrap().position[2] - 0.5).abs() < 1e-5);
        assert_eq!(frame.objects[0].mesh.vertices()[2].position[2], 1.);
    }

    #[test]
    fn attached_camera_lights_and_shadows_reach_the_frame_without_extra_geometry() {
        use crate::{DirectionalShadow, LightKind, PunctualLight};
        let mut graph = SceneGraph::new();
        let root = graph
            .insert(
                None,
                Node::new().transform(AffineTransform::from_translation([2., 0., 0.]).unwrap()),
            )
            .unwrap();
        let camera_node = graph
            .insert(Some(root), Node::new().camera(Camera::default()))
            .unwrap();
        let sun = graph
            .insert(
                Some(root),
                Node::new().light(PunctualLight::directional([0., 1., 0.])),
            )
            .unwrap();
        graph
            .insert(
                Some(root),
                Node::new().light(PunctualLight::point([1., 2., 3.])),
            )
            .unwrap();
        let body = graph
            .insert(
                Some(root),
                Node::new().mesh(Mesh::cube(), Material::color(rgb(0xffffff))),
            )
            .unwrap();
        let evaluated = graph.evaluate().unwrap();
        let scene = evaluated
            .scene_from_camera(camera_node)
            .unwrap()
            .directional_shadow(Some(DirectionalShadow::new([2., 0., 0.], [3.; 3])));
        let frame = scene
            .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        assert_eq!(frame.camera_position, [2., 0., 6.]);
        assert_eq!(
            frame.view_projection,
            scene.camera.view_projection(1.).unwrap()
        );
        let lights = frame.lights.as_ref().unwrap();
        assert_eq!(lights.len(), 2);
        assert_eq!(lights[0].kind, LightKind::Directional);
        assert_eq!(lights[1].position, [3., 2., 3.]);
        assert!(frame.directional_shadow.is_some());
        assert_eq!(frame.objects.len(), 1);
        assert_eq!(frame.objects[0].output_id, 1);
        assert_eq!(scene.objects[0].node, Some(body));
        graph.set_visible(sun, false).unwrap();
        assert_eq!(evaluated.lights().count(), 2);
        let hidden = graph
            .evaluate()
            .unwrap()
            .scene_from_camera(camera_node)
            .unwrap();
        assert_eq!(hidden.lights.as_ref().unwrap().len(), 1);
        assert!(
            hidden
                .directional_shadow(Some(DirectionalShadow::new([0.; 3], [3.; 3])))
                .prepare_frame(1., None, |_, _, _| panic!(
                    "invalid shadow must fail before texture resolution"
                ))
                .is_err()
        );
        for _ in 0..crate::MAX_PUNCTUAL_LIGHTS {
            graph
                .insert(None, Node::new().light(PunctualLight::point([0.; 3])))
                .unwrap();
        }
        let many = graph.evaluate().unwrap();
        assert_eq!(many.lights().count(), crate::MAX_PUNCTUAL_LIGHTS + 1);
        let all = many.scene_from_camera(camera_node).unwrap();
        assert!(
            all.prepare_frame(1., None, |_, _, _| panic!(
                "excess lights must fail before texture resolution"
            ))
            .is_err()
        );
        let selected = all.lights(many.lights().take(1).map(|(_, light)| light));
        assert!(
            selected
                .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
                .is_ok()
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
        let camera = Camera::default()
            .frame_bounds(evaluated.bounds().unwrap(), 0.5, 1.2)
            .unwrap();
        let scene = evaluated.scene(camera);
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
