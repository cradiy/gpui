use anyhow::Result;
use gpui::{MeshTexture3d, Scene3dFrame};

use crate::{Scene, Texture, TextureSlot};

impl Scene {
    pub(super) fn bind_frame(
        &self,
        plan: &Scene3dFrame,
        mut resolve: impl FnMut(usize, TextureSlot, &Texture) -> Result<Option<MeshTexture3d>>,
    ) -> Result<Scene3dFrame> {
        let mut objects = Vec::with_capacity(plan.objects.len());
        for template in plan.objects.iter() {
            let index = template.output_id as usize - 1;
            let material = &self.objects[index].material;
            let mut draw = template.clone();
            let mut ready = true;
            for (slot, map) in material.lighting_textures() {
                match resolve(index, slot, &Texture::Image(map.image.clone()))? {
                    Some(MeshTexture3d::Image(tile)) => {
                        let resolved = Some(gpui::MaterialTexture3d {
                            tile,
                            sampling: map.sampling,
                            uv_set: map.uv_set,
                        });
                        match slot {
                            TextureSlot::MetallicRoughness => {
                                draw.metallic_roughness_texture = resolved
                            }
                            TextureSlot::Emissive => draw.emissive_texture = resolved,
                            TextureSlot::Normal => draw.normal_texture = resolved,
                            TextureSlot::Occlusion => draw.occlusion_texture = resolved,
                            TextureSlot::BaseColor => unreachable!(),
                        }
                    }
                    None => ready = false,
                    _ => anyhow::bail!("object {index}: material maps require atlas images"),
                }
            }
            let Some(texture) = resolve(index, TextureSlot::BaseColor, &material.texture)? else {
                continue;
            };
            if ready {
                draw.texture = texture;
                objects.push(draw);
            }
        }
        let mut frame = plan.clone();
        frame.objects = objects.into();
        Ok(frame)
    }
}
