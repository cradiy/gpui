use crate::{Mesh, NodeHandle, ObjectId, Scene, Texture, TextureSlot};

/// Geometry and active coordinate selections before texture resolution or raster culling.
#[derive(Clone, Copy)]
pub struct GeometryInput<'a> {
    pub output_id: u32,
    pub object_index: usize,
    pub id: Option<&'a ObjectId>,
    pub node: Option<NodeHandle>,
    pub mesh: &'a Mesh,
    /// Base color, metallic-roughness, emissive, normal, and occlusion; inactive slots use zero.
    pub uv_sets: [u32; 5],
}

impl Scene {
    /// Inspects every scene object without loading images, allocating GPU resources,
    /// or evaluating vertices. IDs match preparation and GPU draw bindings. Geometry
    /// and material validity are checked separately during preparation/rendering.
    pub fn geometry_inputs(
        &self,
    ) -> anyhow::Result<impl ExactSizeIterator<Item = GeometryInput<'_>>> {
        anyhow::ensure!(
            self.objects.len() < u32::MAX as usize,
            "too many objects for 32-bit output IDs"
        );
        Ok(self.objects.iter().enumerate().map(|(index, object)| {
            let mut uv_sets = [0; 5];
            if matches!(object.material.texture, Texture::Image(_)) {
                uv_sets[0] = object.material.uv_set;
            }
            for (slot, texture) in object.material.lighting_textures() {
                let index = match slot {
                    TextureSlot::BaseColor => 0,
                    TextureSlot::MetallicRoughness => 1,
                    TextureSlot::Emissive => 2,
                    TextureSlot::Normal => 3,
                    TextureSlot::Occlusion => 4,
                };
                uv_sets[index] = texture.uv_set;
            }
            GeometryInput {
                output_id: index as u32 + 1,
                object_index: index,
                id: object.id.as_ref(),
                node: object.node,
                mesh: &object.mesh,
                uv_sets,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Camera, Material, MaterialTexture, Node, PbrMaterial, SceneGraph, TextureSource,
        TextureState,
    };
    use gpui::{
        AtlasTextureId, AtlasTextureKind, AtlasTile, Bounds, DevicePixels, MeshTexture3d, point,
        size,
    };

    #[test]
    fn geometry_inputs_match_render_coordinates_before_images_are_ready() {
        let mut mesh = Mesh::plane();
        for set in 1..=4 {
            mesh = mesh
                .with_uv_set(
                    set,
                    mesh.vertices().iter().map(|vertex| vertex.uv).collect(),
                )
                .unwrap();
        }
        mesh = mesh
            .with_tangents_for_uv_set(4, vec![[1., 0., 0., 1.]; mesh.vertices().len()])
            .unwrap();
        let material = Material::image("base.png")
            .image_uv_set(1)
            .pbr(PbrMaterial::default())
            .metallic_roughness_texture(MaterialTexture::new("roughness.png").uv_set(2))
            .emissive_texture(MaterialTexture::new("emission.png").uv_set(3))
            .normal_texture(MaterialTexture::new("normal.png").uv_set(4))
            .occlusion_texture(MaterialTexture::new("occlusion.png").uv_set(2));
        let mut custom = material.clone().unlit(true);
        custom.pbr = None;
        custom.custom_material = Some(gpui::MeshMaterial3d::new(std::sync::Arc::new(())));
        let mut extra = custom.clone();
        extra.mesh_passes = vec![gpui::MeshPass3d {
            material: extra.custom_material.take().unwrap(),
            state: Default::default(),
            expansion: None,
        }]
        .into();
        let mut graph = SceneGraph::new();
        for material in [
            material.clone(),
            material.clone().unlit(true),
            material.normal_scale(0.).occlusion_strength(0.),
            custom,
            extra,
            Material::ui().image_uv_set(3),
        ] {
            graph
                .insert(None, Node::new().mesh(mesh.clone(), material))
                .unwrap();
        }
        let scene = graph.evaluate().unwrap().scene(Camera::default());
        let inputs: Vec<_> = scene.geometry_inputs().unwrap().collect();
        assert_eq!(
            inputs.iter().map(|input| input.uv_sets).collect::<Vec<_>>(),
            vec![
                [1, 2, 3, 4, 2],
                [1, 0, 0, 0, 0],
                [1, 2, 3, 0, 0],
                [1, 2, 3, 4, 2],
                [1, 2, 3, 4, 2],
                [0; 5],
            ]
        );
        let pending = scene
            .prepare(1., None, |_| Ok(TextureState::Pending))
            .unwrap();
        assert!(pending.frame().objects.is_empty());
        assert_eq!(pending.objects().len(), inputs.len());
        let tile = AtlasTile {
            texture_id: AtlasTextureId {
                index: 0,
                kind: AtlasTextureKind::Polychrome,
            },
            tile_id: gpui::TileId(0),
            padding: 0,
            bounds: Bounds::new(
                point(DevicePixels(0), DevicePixels(0)),
                size(DevicePixels(1), DevicePixels(1)),
            ),
        };
        let ready = scene
            .prepare(1., None, |request| {
                Ok(TextureState::Ready(match request.source {
                    TextureSource::Solid => MeshTexture3d::None,
                    TextureSource::Ui => MeshTexture3d::Subtree,
                    TextureSource::Image(_) => MeshTexture3d::Image(tile),
                }))
            })
            .unwrap();
        assert_eq!(ready.frame().objects.len(), inputs.len());
        for (input, rendered) in inputs.iter().zip(ready.frame().objects.iter()) {
            assert_eq!(input.output_id, rendered.output_id);
            assert_eq!(input.node, ready.object(rendered.output_id).unwrap().node);
            assert_eq!(input.uv_sets, rendered.texture_uv_sets());
            assert!(std::sync::Arc::ptr_eq(&input.mesh.0, &rendered.mesh));
        }
    }
}
