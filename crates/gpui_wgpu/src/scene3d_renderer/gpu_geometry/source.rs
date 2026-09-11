use super::*;

impl WgpuScene3dGeometry {
    /// Uploads a mesh and coordinate selection using this source's packing kernels.
    /// Index storage is shared only when both meshes reference the same index slice.
    /// UV/color values come from the supplied mesh; prior stream updates are not inherited.
    /// Earlier sources and results remain unchanged. The limit admits the complete
    /// replacement source plus one result, including any shared index allocation.
    pub fn with_mesh(
        &self,
        mesh: Arc<Mesh3d>,
        uv_sets: [u32; 5],
        byte_limit: Option<u64>,
    ) -> Result<Self> {
        let (memory, source, indices) = upload(
            &self.context,
            &mesh,
            uv_sets,
            byte_limit,
            Some((&self.mesh, &self.indices)),
        )?;
        Ok(Self {
            context: self.context.clone(),
            mesh,
            uv_sets,
            memory,
            source,
            indices,
            layout: self.layout.clone(),
            pipeline: self.pipeline.clone(),
            attribute_kernel: self.attribute_kernel.clone(),
        })
    }
}

pub(super) fn upload(
    context: &WgpuContext,
    mesh: &Mesh3d,
    uv_sets: [u32; 5],
    byte_limit: Option<u64>,
    previous: Option<(&Mesh3d, &wgpu::Buffer)>,
) -> Result<(Scene3dGpuGeometryMemory, wgpu::Buffer, wgpu::Buffer)> {
    ensure!(!context.device_lost(), "GPU geometry device is lost");
    for set in uv_sets {
        ensure!(
            mesh.uv_at(set, 0).is_some(),
            "GPU geometry missing UV set {set}"
        );
    }
    let memory = Scene3dGpuGeometryMemory::plan(mesh.vertices().len(), mesh.indices().len())?;
    let device = &context.device;
    memory.validate(
        &device.limits(),
        mesh.vertices().len(),
        mesh.indices().len(),
        byte_limit,
    )?;
    let vertices: Vec<_> = (0..mesh.vertices().len())
        .map(|index| Vertex::new(mesh, index, uv_sets))
        .collect();
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let source = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("scene3d.geometry.source"),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
    });
    let indices = match previous.filter(|(old, _)| std::ptr::eq(old.indices(), mesh.indices())) {
        Some((_, buffer)) => buffer.clone(),
        None => device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("scene3d.geometry.indices"),
            contents: bytemuck::cast_slice(mesh.indices()),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::INDEX,
        }),
    };
    if let Some(error) = gpui::block_on(scope.pop()) {
        anyhow::bail!("GPU geometry source upload: {error}");
    }
    Ok((memory, source, indices))
}
