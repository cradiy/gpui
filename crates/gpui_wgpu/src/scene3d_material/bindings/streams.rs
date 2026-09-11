use super::*;
use crate::Scene3dVertexAttribute;

#[cfg(test)]
mod tests;

struct Streams {
    source: Scene3dMaterialSource,
    vertex_count: usize,
    buffers: Vec<wgpu::Buffer>,
    bind_group: wgpu::BindGroup,
    payload_bytes: u64,
}

/// Immutable custom-attribute buffers in mesh vertex order, retaining their source layout.
/// Updates share unchanged allocations and never overwrite buffers used by earlier frames.
#[derive(Clone)]
pub struct Scene3dVertexStreams(Arc<Streams>);

impl Scene3dVertexStreams {
    pub fn source(&self) -> &Scene3dMaterialSource {
        &self.0.source
    }
    pub fn vertex_count(&self) -> usize {
        self.0.vertex_count
    }
    /// Complete GPU payload, including buffers shared with other versions.
    pub fn payload_bytes(&self) -> u64 {
        self.0.payload_bytes
    }
    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.0.bind_group
    }

    /// Replaces named streams; omitted streams retain their buffers. Inputs are copied.
    /// The budget admits the complete snapshot, not only changed streams. Empty updates
    /// share the snapshot after validating device health and the supplied budget.
    pub fn with_values(&self, values: &[(&str, &[u8])], max_payload_bytes: u64) -> Result<Self> {
        self.source().build_vertex_streams(
            self.vertex_count(),
            values,
            max_payload_bytes,
            Some(self),
        )
    }
}

impl Scene3dMaterialSource {
    /// Uploads every declared custom vertex stream exactly once, in arbitrary name order.
    /// Each value has exactly `vertex_count * format.size()` tightly packed bytes.
    /// Floating-point lanes must be finite; integer lanes preserve all 32 bits.
    /// The byte budget covers the complete GPU snapshot, excluding retained versions,
    /// material uniforms, caller input storage, and driver overhead.
    pub fn bind_vertex_streams(
        &self,
        vertex_count: usize,
        values: &[(&str, &[u8])],
        max_payload_bytes: u64,
    ) -> Result<Scene3dVertexStreams> {
        self.build_vertex_streams(vertex_count, values, max_payload_bytes, None)
    }

    fn build_vertex_streams(
        &self,
        vertex_count: usize,
        values: &[(&str, &[u8])],
        max_payload_bytes: u64,
        previous: Option<&Scene3dVertexStreams>,
    ) -> Result<Scene3dVertexStreams> {
        ensure!(!self.context().device_lost(), "material device is lost");
        let declarations = self.program().vertex_attributes();
        let plan = StreamPlan::new(
            declarations,
            vertex_count,
            values,
            previous.is_some(),
            &self.context().device.limits(),
            max_payload_bytes,
        )?;
        if values.is_empty() {
            if let Some(previous) = previous {
                return Ok(previous.clone());
            }
        }
        let device = &self.context().device;
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let buffers: Vec<_> = plan
            .mapping
            .iter()
            .enumerate()
            .map(|(index, supplied)| match supplied {
                Some(value) => device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("gpui_3d.material.vertex_stream"),
                    contents: values[*value].1,
                    usage: wgpu::BufferUsages::STORAGE,
                }),
                None => previous.expect("validated partial stream update").0.buffers[index].clone(),
            })
            .collect();
        let entries: Vec<_> = buffers
            .iter()
            .enumerate()
            .map(|(binding, buffer)| wgpu::BindGroupEntry {
                binding: binding as u32,
                resource: buffer.as_entire_binding(),
            })
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpui_3d.material.vertex_streams"),
            layout: self.vertex_layout().expect("validated vertex declarations"),
            entries: &entries,
        });
        if let Some(error) = gpui::block_on(scope.pop()) {
            anyhow::bail!("material vertex binding: {error}");
        }
        ensure!(!self.context().device_lost(), "material device is lost");
        Ok(Scene3dVertexStreams(Arc::new(Streams {
            source: self.clone(),
            vertex_count,
            buffers,
            bind_group,
            payload_bytes: plan.payload_bytes,
        })))
    }
}

struct StreamPlan {
    mapping: Vec<Option<usize>>,
    payload_bytes: u64,
}

impl StreamPlan {
    fn new(
        declarations: &[Scene3dVertexAttribute],
        count: usize,
        values: &[(&str, &[u8])],
        partial: bool,
        limits: &wgpu::Limits,
        max_bytes: u64,
    ) -> Result<Self> {
        ensure!(
            !declarations.is_empty(),
            "material does not declare custom vertex streams"
        );
        ensure!(
            count > 0 && count <= u32::MAX as usize / 4,
            "custom vertex stream addressing exceeds u32"
        );
        ensure!(
            values.len() <= declarations.len(),
            "too many custom vertex streams"
        );
        let mut payload_bytes = 0_u64;
        for attribute in declarations {
            let bytes = count as u64 * attribute.format.size();
            ensure!(
                bytes <= limits.max_buffer_size && bytes <= limits.max_storage_buffer_binding_size,
                "custom vertex stream {} exceeds device buffer limits",
                attribute.name
            );
            payload_bytes = payload_bytes
                .checked_add(bytes)
                .ok_or_else(|| anyhow::anyhow!("custom vertex payload overflow"))?;
        }
        ensure!(
            payload_bytes <= max_bytes,
            "custom vertex streams exceed snapshot payload budget"
        );
        let mut mapping = vec![None; declarations.len()];
        for (value, (name, bytes)) in values.iter().enumerate() {
            let index = declarations
                .iter()
                .position(|attribute| attribute.name == *name)
                .ok_or_else(|| anyhow::anyhow!("unknown custom vertex stream {name}"))?;
            ensure!(
                mapping[index].is_none(),
                "duplicate custom vertex stream {name}"
            );
            let attribute = &declarations[index];
            let expected = count as u64 * attribute.format.size();
            ensure!(
                bytes.len() as u64 == expected,
                "custom vertex stream {name} requires exactly {expected} bytes"
            );
            if matches!(
                attribute.format,
                wgpu::VertexFormat::Float32
                    | wgpu::VertexFormat::Float32x2
                    | wgpu::VertexFormat::Float32x3
                    | wgpu::VertexFormat::Float32x4
            ) {
                ensure!(
                    bytes
                        .chunks_exact(4)
                        .all(|word| f32::from_ne_bytes(word.try_into().unwrap()).is_finite()),
                    "custom vertex stream {name} contains nonfinite floats"
                );
            }
            mapping[index] = Some(value);
        }
        ensure!(
            partial || mapping.iter().all(Option::is_some),
            "missing custom vertex streams"
        );
        Ok(Self {
            mapping,
            payload_bytes,
        })
    }
}
