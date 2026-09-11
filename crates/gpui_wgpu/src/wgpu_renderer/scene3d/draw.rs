use super::Scene3dRenderer;
use crate::scene3d_renderer::gpu_draws::GpuGeometryMap;

impl Scene3dRenderer {
    pub(crate) fn set_gpu_geometry(&mut self, geometry: &GpuGeometryMap) {
        self.gpu_geometry.clone_from(geometry);
    }

    pub(super) fn draw_geometry(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        object: &gpui::MeshDraw3d,
        instances: u32,
    ) {
        if let Some(geometry) = self.gpu_geometry.get(&object.output_id) {
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
