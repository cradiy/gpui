use crate::{Scene, Texture};
use anyhow::{Result, ensure};
use gpui::{MeshDraw3d, MeshTexture3d, Scene3dFrame, UiTexture3d};

impl Scene {
    pub(crate) fn prepare_frame(
        &self,
        aspect: f32,
        ui_texture: Option<UiTexture3d>,
        mut resolve: impl FnMut(usize, &Texture) -> Result<Option<MeshTexture3d>>,
    ) -> Result<Scene3dFrame> {
        let view_projection = self.camera.view_projection(aspect)?;
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
            let Some(texture) = resolve(index, &object.material.texture)? else {
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
                alpha_cutoff: object.material.alpha_cutoff,
                unlit: object.material.unlit,
            });
        }
        Ok(Scene3dFrame {
            ui_texture,
            view_projection,
            camera_position: self.camera.eye,
            orthographic_view_direction: match self.camera.projection {
                crate::Projection::Perspective { .. } => None,
                crate::Projection::Orthographic { .. } => {
                    let view = self.camera.view_matrix()?;
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
            .prepare_frame(1.5, None, |_, _| Ok(Some(MeshTexture3d::None)))
            .unwrap();
        let b = scene
            .prepare_frame(0.5, None, |_, _| Ok(Some(MeshTexture3d::None)))
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
            .prepare_frame(1., None, |_, texture| {
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
                .prepare_frame(0., None, |_, _| unreachable!())
                .is_err()
        );
        let invalid = Scene::new()
            .object(Object::new(Mesh::cube(), Material::color(rgb(0xffffff))).scale([0., 1., 1.]));
        assert!(
            invalid
                .prepare_frame(1., None, |_, _| unreachable!())
                .is_err()
        );
        assert!(
            scene
                .prepare_frame(1., None, |_, _| anyhow::bail!("image unavailable"))
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
                    .prepare_frame(1., None, |_, _| unreachable!())
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
                .prepare_frame(1., None, |_, _| Ok(Some(MeshTexture3d::None)))
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
                    .prepare_frame(1., None, |_, _| unreachable!())
                    .is_err()
            );
        }
    }
}
