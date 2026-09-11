use super::Scene3dRenderer;
use crate::Scene3dGpuGeometry;

impl Scene3dRenderer {
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
