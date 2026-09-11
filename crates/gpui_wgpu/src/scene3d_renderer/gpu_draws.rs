use std::sync::Arc;

use anyhow::{Result, ensure};
use collections::{HashMap, HashSet};
use gpui::Scene3dFrame;

use super::{Scene3dChannels, Scene3dGeometryMemory, Scene3dGpuGeometry};
use crate::WgpuContext;

pub(crate) type GpuGeometryMap = HashMap<u32, Arc<Scene3dGpuGeometry>>;

/// One frame-local GPU geometry override with caller-supplied conservative local bounds.
/// Does not update the source mesh, CPU queries, or objects omitted from the input frame.
#[derive(Clone)]
pub struct Scene3dGpuDraw {
    pub output_id: u32,
    pub geometry: Arc<Scene3dGpuGeometry>,
    pub bounds: [[f32; 3]; 2],
}

pub(super) fn prepare(
    context: &WgpuContext,
    frame: &Scene3dFrame,
    draws: &[Scene3dGpuDraw],
) -> Result<(Scene3dFrame, GpuGeometryMap)> {
    let mut frame = with_bounds(
        frame,
        draws.iter().map(|draw| (draw.output_id, draw.bounds)),
    )?;
    for draw in draws {
        let object = Arc::make_mut(&mut frame.objects)
            .iter_mut()
            .find(|object| object.output_id == draw.output_id)
            .unwrap();
        object.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(draw.geometry.clone()));
    }
    let geometry = validate_frame(&context.device, &frame)?;
    Ok((frame, geometry))
}

pub(crate) fn validate_frame(
    device: &wgpu::Device,
    frame: &Scene3dFrame,
) -> Result<GpuGeometryMap> {
    let geometry = frame_geometry(frame)?;
    for gpu in geometry.values() {
        ensure!(
            !gpu.context().device_lost() && std::ptr::eq(device, gpu.context().device.as_ref()),
            "GPU geometry belongs to a different or lost device"
        );
    }
    Ok(geometry)
}

pub(super) fn frame_geometry(frame: &Scene3dFrame) -> Result<GpuGeometryMap> {
    let mut geometry = HashMap::default();
    if !frame
        .objects
        .iter()
        .any(|object| object.gpu_geometry.is_some())
    {
        return Ok(geometry);
    }
    let mut seen = HashSet::default();
    for object in frame.objects.iter() {
        ensure!(
            object.output_id != 0 && seen.insert(object.output_id),
            "GPU draw routing requires unique nonzero object IDs"
        );
        let Some(resource) = &object.gpu_geometry else {
            continue;
        };
        let gpu = resource
            .downcast::<Scene3dGpuGeometry>()
            .ok_or_else(|| anyhow::anyhow!("unsupported GPU geometry backend"))?;
        ensure!(
            object.render_bounds.is_some_and(|bounds| bounds
                .iter()
                .flatten()
                .all(|v| v.is_finite())
                && (0..3).all(|axis| bounds[0][axis] <= bounds[1][axis])),
            "GPU geometry requires finite ordered render bounds"
        );
        ensure!(
            Arc::ptr_eq(&object.mesh, gpu.base_mesh()),
            "GPU geometry source mesh mismatch for object {}",
            object.output_id
        );
        ensure!(
            object.texture_uv_sets() == gpu.uv_sets(),
            "GPU geometry material coordinates mismatch for object {}",
            object.output_id
        );
        geometry.insert(object.output_id, gpu);
    }
    Ok(geometry)
}

fn with_bounds(
    frame: &Scene3dFrame,
    bounds: impl IntoIterator<Item = (u32, [[f32; 3]; 2])>,
) -> Result<Scene3dFrame> {
    let mut bounds = bounds.into_iter().peekable();
    if bounds.peek().is_none() {
        return Ok(frame.clone());
    }
    let mut frame = frame.clone();
    let mut indices = HashMap::default();
    for (index, object) in frame.objects.iter().enumerate() {
        ensure!(
            object.output_id != 0 && indices.insert(object.output_id, index).is_none(),
            "GPU draw routing requires unique nonzero object IDs"
        );
    }
    let mut seen = HashSet::default();
    for (id, bounds) in bounds {
        let &index = indices.get(&id).ok_or_else(|| {
            anyhow::anyhow!("GPU draw object {id} is absent from the prepared frame")
        })?;
        ensure!(seen.insert(id), "duplicate GPU draw object {id}");
        ensure!(
            bounds.iter().flatten().all(|v| v.is_finite())
                && (0..3).all(|axis| bounds[0][axis] <= bounds[1][axis]),
            "GPU draw object {id} has invalid render bounds"
        );
        let object = &mut Arc::make_mut(&mut frame.objects)[index];
        object.render_bounds = Some(bounds);
        if object.alpha_mode == gpui::AlphaMode3d::Blend {
            let center: [f64; 4] = std::array::from_fn(|axis| {
                if axis == 3 {
                    1.
                } else {
                    (f64::from(bounds[0][axis]) + f64::from(bounds[1][axis])) * 0.5
                }
            });
            let world: [f64; 4] = std::array::from_fn(|row| {
                (0..4)
                    .map(|col| f64::from(object.model[col][row]) * center[col])
                    .sum()
            });
            object.sort_depth = -(0..4)
                .map(|col| f64::from(frame.world_to_view[col][2]) * world[col])
                .sum::<f64>();
            ensure!(
                object.sort_depth.is_finite(),
                "GPU draw object {id} has invalid sort depth"
            );
        }
    }
    Ok(frame)
}

pub(super) fn memory(
    frame: &Scene3dFrame,
    channels: Scene3dChannels,
    geometry: &GpuGeometryMap,
) -> Result<Scene3dGeometryMemory> {
    if geometry.is_empty() {
        return Scene3dGeometryMemory::plan(frame, channels);
    }
    let mut cpu = frame.clone();
    cpu.objects = frame
        .objects
        .iter()
        .filter(|object| !geometry.contains_key(&object.output_id))
        .cloned()
        .collect();
    let mut memory = Scene3dGeometryMemory::plan(&cpu, channels)?;
    let mut seen = HashSet::default();
    #[expect(
        clippy::mutable_key_type,
        reason = "wgpu buffers hash their immutable backend identity"
    )]
    let mut seen_indices = HashSet::default();
    for object in frame.objects.iter() {
        let Some(geometry) = geometry.get(&object.output_id) else {
            continue;
        };
        let active = object.intersects_clip_volume(frame.view_projection)
            || (channels.shaded()
                && object.cast_shadows
                && object.alpha_mode != gpui::AlphaMode3d::Blend
                && frame
                    .directional_shadow
                    .is_some_and(|shadow| object.intersects_clip_volume(shadow.view_projection)));
        if !active || !seen.insert(Arc::as_ptr(geometry)) {
            continue;
        }
        let packed = geometry.memory();
        let index_bytes = if seen_indices.insert(geometry.indices()) {
            packed.index_bytes
        } else {
            0
        };
        memory.meshes += 1;
        memory.vertex_bytes += packed.vertex_bytes;
        memory.index_bytes += index_bytes;
        memory.indirect_bytes += packed.draw_bytes;
        memory.total_bytes += packed.vertex_bytes + index_bytes + packed.draw_bytes;
        let largest = packed
            .vertex_bytes
            .max(packed.index_bytes)
            .max(packed.draw_bytes);
        if largest > memory.max_buffer_bytes {
            memory.max_buffer_bytes = largest;
            memory.max_buffer_object_id = Some(object.output_id);
        }
    }
    Ok(memory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wgpu_renderer::scene3d::tests::{frame, object};

    #[test]
    fn empty_bounds_overrides_preserve_cpu_frames_and_nonempty_overrides_validate_ids() {
        let mut first = object();
        first.output_id = 0;
        let source = frame(&[first.clone(), first]);
        let unchanged = with_bounds(&source, []).unwrap();
        assert!(Arc::ptr_eq(&unchanged.objects, &source.objects));
        assert!(frame_geometry(&unchanged).unwrap().is_empty());
        assert_eq!(
            Scene3dGeometryMemory::plan(&unchanged, Scene3dChannels::COLOR).unwrap(),
            Scene3dGeometryMemory::plan(&source, Scene3dChannels::COLOR).unwrap()
        );
        assert!(with_bounds(&source, [(0, [[0.; 3], [1.; 3]])]).is_err());
        let mut duplicate = source;
        for object in Arc::make_mut(&mut duplicate.objects) {
            object.output_id = 1;
        }
        assert!(with_bounds(&duplicate, [(1, [[0.; 3], [1.; 3]])]).is_err());
    }

    #[test]
    fn geometry_planning_rejects_foreign_payloads_instead_of_counting_cpu_fallbacks() {
        let mut gpu = object();
        gpu.gpu_geometry = Some(gpui::MeshGpuGeometry3d::new(Arc::new(())));
        gpu.render_bounds = Some([[0.; 3], [1.; 3]]);
        let invalid = frame(&[gpu]);
        assert!(
            Scene3dGeometryMemory::plan(&invalid, Scene3dChannels::COLOR)
                .unwrap_err()
                .to_string()
                .contains("unsupported GPU geometry backend")
        );
        let original = frame(&[object()]);
        let memory = Scene3dGeometryMemory::plan(&original, Scene3dChannels::COLOR).unwrap();
        assert_eq!(memory.indirect_bytes, 0);
        assert!(memory.vertex_bytes > 0);
    }

    #[test]
    #[ignore = "requires a compute-capable GPU"]
    fn gpu_draws_match_cpu_geometry_across_channels_and_preserve_independent_instances()
    -> Result<()> {
        use crate::{
            Scene3dGpuOutput, Scene3dOutputConfig, Scene3dPixels, WgpuScene3dGeometry,
            WgpuScene3dRenderer,
        };
        use wgpu::util::DeviceExt as _;
        let context = WgpuContext::new_headless()?;
        let mut renderer = WgpuScene3dRenderer::new(context.clone())?;
        let first = object();
        let mut second = first.clone();
        second.output_id = 2;
        second.color = gpui::rgb(0x6699ff);
        let source = frame(&[first, second]);
        let mut reference = source.clone();
        let template = WgpuScene3dGeometry::new(
            context.clone(),
            source.objects[0].mesh.clone(),
            [0; 5],
            Some(4096),
        )?;
        let mut draws = Vec::new();
        for (index, shift) in [-0.75, 0.125].into_iter().enumerate() {
            let mesh = &source.objects[index].mesh;
            let mut vertices = mesh.vertices().to_vec();
            for v in &mut vertices {
                v.position[0] += shift;
                v.position[2] += 0.25;
            }
            let attributes: Vec<[u32; 16]> = vertices
                .iter()
                .map(|v| {
                    let mut record = [0; 16];
                    for axis in 0..3 {
                        record[axis] = v.position[axis].to_bits();
                        record[4 + axis] = v.normal[axis].to_bits();
                    }
                    record
                })
                .collect();
            let buffer = context
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: None,
                    contents: bytemuck::cast_slice(&attributes),
                    usage: wgpu::BufferUsages::STORAGE,
                });
            let geometry = Arc::new(template.evaluate(&buffer)?);
            draws.push(Scene3dGpuDraw {
                output_id: index as u32 + 1,
                geometry,
                bounds: [[shift, 0., 0.25], [shift + 1., 1., 0.25]],
            });
            Arc::make_mut(&mut reference.objects)[index].mesh =
                mesh.with_vertices(vertices, None)?;
        }
        let config = Scene3dOutputConfig {
            size: [64, 64],
            channels: Scene3dChannels::all(),
            color_samples: 1,
        };
        let actual = renderer.render_with_geometry(&source, config, &draws)?;
        let expected = renderer.render(&reference, config)?;
        let read = |output: &Scene3dGpuOutput| -> Result<Scene3dPixels> {
            let mut pending = output.readback()?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                if let Some(pixels) = pending.try_read()? {
                    return Ok(pixels);
                }
                ensure!(
                    std::time::Instant::now() < deadline,
                    "GPU draw readback timed out"
                );
                std::thread::yield_now();
            }
        };
        let actual_pixels = read(&actual)?;
        let expected_pixels = read(&expected)?;
        assert_eq!(actual_pixels.object_ids, expected_pixels.object_ids);
        assert!(actual_pixels.object_ids.as_ref().unwrap().contains(&1));
        assert!(actual_pixels.object_ids.as_ref().unwrap().contains(&2));
        assert_eq!(actual_pixels.rgba, expected_pixels.rgba);
        assert_eq!(actual_pixels.linear_rgba, expected_pixels.linear_rgba);
        assert_eq!(actual_pixels.linear_depth, expected_pixels.linear_depth);
        assert_eq!(actual_pixels.world_normals, expected_pixels.world_normals);
        assert_eq!(actual.geometry_memory().indirect_bytes, 40);
        assert_eq!(
            actual.geometry_memory().index_bytes,
            template.memory().index_bytes
        );
        assert_eq!(
            actual.geometry_memory().vertex_bytes,
            template.memory().vertex_bytes * 2
        );
        let (attached, _) = prepare(&context, &source, &draws)?;
        assert_eq!(
            Scene3dGeometryMemory::plan(&attached, config.channels)?,
            actual.geometry_memory()
        );
        assert_eq!(
            read(&renderer.render(&attached, config)?)?.object_ids,
            actual_pixels.object_ids
        );
        renderer.set_geometry_byte_limit(Some(actual.geometry_memory().total_bytes - 1));
        assert!(
            renderer
                .render_with_geometry(&source, config, &draws)
                .is_err()
        );
        renderer.set_geometry_byte_limit(None);
        let (_, routed) = prepare(&context, &source, &draws)?;
        let (with_bounds, _) = prepare(&context, &source, &draws)?;
        assert_eq!(
            memory(&with_bounds, config.channels, &routed)?,
            actual.geometry_memory()
        );
        assert!(
            renderer
                .render_with_geometry(&source, config, &[draws[0].clone(), draws[0].clone()])
                .is_err()
        );
        drop((renderer, template, draws));
        assert_eq!(read(&actual)?.rgba, actual_pixels.rgba);
        Ok(())
    }

    #[test]
    fn render_bounds_route_shared_mesh_instances_without_changing_cpu_geometry() {
        let first = object();
        let mut second = first.clone();
        second.output_id = 2;
        second.alpha_mode = gpui::AlphaMode3d::Blend;
        let original = frame(&[first, second]);
        let updated = with_bounds(
            &original,
            [
                (1, [[3., 0., 0.], [4., 1., 1.]]),
                (2, [[0., 0., -5.], [1., 1., -3.]]),
            ],
        )
        .unwrap();
        assert!(!updated.objects[0].intersects_clip_volume(updated.view_projection));
        assert!(original.objects[0].intersects_clip_volume(original.view_projection));
        assert_eq!(updated.objects[1].sort_depth, 4.);
        assert!(Arc::ptr_eq(
            &updated.objects[0].mesh,
            &original.objects[0].mesh
        ));
        assert!(original.objects.iter().all(|o| o.render_bounds.is_none()));
        assert!(with_bounds(&original, [(3, [[0.; 3]; 2])]).is_err());
        assert!(with_bounds(&original, [(1, [[0.; 3]; 2]), (1, [[0.; 3]; 2])]).is_err());
        assert!(with_bounds(&original, [(1, [[1.; 3], [0.; 3]])]).is_err());
        assert!(with_bounds(&original, [(1, [[f32::NAN; 3]; 2])]).is_err());
        let duplicate = frame(&[original.objects[0].clone(), original.objects[0].clone()]);
        assert!(with_bounds(&duplicate, [(1, [[0.; 3]; 2])]).is_err());
    }
}
