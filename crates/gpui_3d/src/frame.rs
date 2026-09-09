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
                alpha_cutoff: object.material.alpha_cutoff,
                unlit: object.material.unlit,
            });
        }
        Ok(Scene3dFrame {
            ui_texture,
            view_projection,
            light_direction: light.direction,
            light: [
                light.color.r,
                light.color.g,
                light.color.b,
                light.intensity.max(0.),
            ],
            ambient: light.ambient.max(0.),
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
    }
}
