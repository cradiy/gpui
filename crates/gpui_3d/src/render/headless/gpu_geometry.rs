use std::collections::HashSet;
use std::sync::Arc;

use super::{HeadlessRenderer, RenderedFrame};
use crate::render::gpu_geometry::with_bounds;
use crate::{Scene, Scene3dGpuDraw, Scene3dOutputConfig};
use anyhow::{Result, ensure};

impl HeadlessRenderer {
    /// Renders GPU geometry with conservative local bounds, without vertex readback.
    /// IDs address scene object indices plus one, including objects outside the camera.
    /// All output channels and shadows use the same geometry. Bounds only affect rendering;
    /// source scene CPU queries remain in their original pose. Use output ID/depth readback
    /// for screen-space selection, or explicitly materialize meshes for CPU spatial queries.
    pub fn render_with_geometry(
        &mut self,
        scene: &Scene,
        config: Scene3dOutputConfig,
        draws: &[Scene3dGpuDraw],
    ) -> Result<RenderedFrame> {
        if draws.is_empty() {
            return self.render(scene, config);
        }
        self.renderer.validate_target_memory(
            config,
            scene.directional_shadow.map(|shadow| shadow.resolution),
        )?;
        let render_scene = with_bounds(
            scene,
            draws.iter().map(|draw| (draw.output_id, draw.bounds)),
        )?;
        for draw in draws {
            let object = &render_scene.objects[draw.output_id as usize - 1];
            ensure!(
                Arc::ptr_eq(&self.context().device, &draw.geometry.context().device),
                "GPU geometry belongs to a different device"
            );
            ensure!(
                Arc::ptr_eq(&object.mesh.0, draw.geometry.base_mesh()),
                "GPU geometry source mesh mismatch for object {}",
                draw.output_id
            );
        }
        let atlas = self.renderer.sprite_atlas();
        let max_dimension = self.capabilities().max_dimension;
        let prepared = self.images.prepare(
            &mut self.preparation,
            &render_scene,
            config.size[0] as f32 / config.size[1] as f32,
            max_dimension,
            atlas.as_ref(),
        )?;
        let visible: HashSet<_> = prepared
            .frame()
            .objects
            .iter()
            .map(|object| object.output_id)
            .collect();
        let active: Vec<_> = draws
            .iter()
            .filter(|draw| visible.contains(&draw.output_id))
            .cloned()
            .collect();
        let output = self
            .renderer
            .render_with_geometry(prepared.frame(), config, &active)?;
        Ok(RenderedFrame {
            output,
            objects: prepared.identities(),
            camera: scene.camera,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Material, Mesh, Object};
    use gpui::MeshTexture3d;

    #[test]
    fn deformed_render_bounds_recover_culled_objects_without_moving_cpu_meshes() {
        let source = Scene::new().object(
            Object::new(Mesh::cube(), Material::color(gpui::rgb(0xffffff)))
                .position([100., 0., 0.]),
        );
        let plan = |scene: &Scene| {
            scene
                .prepare_frame(1., None, |_, _, _| Ok(Some(MeshTexture3d::None)))
                .unwrap()
        };
        assert!(plan(&source).objects.is_empty());
        let moved = with_bounds(&source, [(1, [[-100.5, -0.5, -0.5], [-99.5, 0.5, 0.5]])]).unwrap();
        assert_eq!(plan(&moved).objects.len(), 1);
        assert_eq!(
            source.objects[0].mesh.bounds(),
            moved.objects[0].mesh.bounds()
        );
        assert!(source.objects[0].render_bounds.is_none());
        assert!(with_bounds(&source, [(0, [[0.; 3]; 2])]).is_err());
        assert!(with_bounds(&source, [(2, [[0.; 3]; 2])]).is_err());
        assert!(with_bounds(&source, [(1, [[1.; 3], [0.; 3]])]).is_err());
    }
}
