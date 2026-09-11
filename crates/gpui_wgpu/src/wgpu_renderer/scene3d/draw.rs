use super::Scene3dRenderer;
use crate::Scene3dGpuGeometry;

impl Scene3dRenderer {
    pub(super) fn bind_material_pipeline(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        object: &gpui::MeshDraw3d,
        shadow: bool,
    ) {
        #[cfg(not(target_family = "wasm"))]
        if let Some(material) = super::materials::snapshot(object).expect("validated material") {
            let pipelines = self.materials.get(material);
            let pipeline = if shadow {
                pipelines.shadow.as_ref().unwrap()
            } else if object.alpha_mode == gpui::AlphaMode3d::Blend {
                pipelines.blend.as_ref().unwrap_or(&pipelines.mesh)
            } else {
                &pipelines.mesh
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(1, material.bind_group(), &[]);
            if let Some(streams) = material.vertex_streams() {
                pass.set_bind_group(2, streams.bind_group(), &[]);
            }
            return;
        }
        let pipeline = if shadow {
            self.shadow_pipeline.as_ref().unwrap()
        } else if object.alpha_mode == gpui::AlphaMode3d::Blend {
            self.blend_pipeline.as_ref().unwrap_or(&self.pipeline)
        } else {
            &self.pipeline
        };
        pass.set_pipeline(pipeline);
    }

    pub(super) fn draw_geometry(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        object: &gpui::MeshDraw3d,
        instances: u32,
    ) {
        if let Some(resource) = &object.gpu_geometry {
            let geometry = resource
                .downcast_ref::<Scene3dGpuGeometry>()
                .expect("GPU geometry must be validated before drawing");
            debug_assert_eq!(instances, 1);
            pass.set_vertex_buffer(0, geometry.vertices().slice(..));
            pass.set_index_buffer(geometry.indices().slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed_indirect(geometry.draw(), 0);
        } else {
            let geometry = self.geometry.get(&object.mesh, object.texture_uv_sets());
            pass.set_vertex_buffer(0, geometry.vertices.slice(..));
            pass.set_index_buffer(geometry.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..geometry.count, 0, 0..instances);
        }
    }
}
