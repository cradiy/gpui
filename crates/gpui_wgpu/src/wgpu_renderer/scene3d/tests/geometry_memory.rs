use super::*;
use crate::{Scene3dChannels as C, Scene3dGeometryMemory as Memory};

#[test]
fn geometry_memory_deduplicates_instances_and_output_channels() {
    let source = object();
    let input = frame(&vec![source.clone(); 7]);
    let one = Memory::plan(&input, C::COLOR).unwrap();
    assert_eq!(
        (
            one.meshes,
            one.vertex_bytes,
            one.index_bytes,
            one.total_bytes
        ),
        (1, 288, 12, 300)
    );
    assert_eq!(Memory::plan(&input, C::all()).unwrap(), one);
    assert_eq!(Memory::plan(&input, C::WORLD_NORMAL).unwrap(), one);
    let different = frame(&[source, object()]);
    let two = Memory::plan(&different, C::COLOR).unwrap();
    assert_eq!((two.meshes, two.total_bytes), (2, 600));
}

#[test]
fn geometry_memory_uses_the_uploaded_material_coordinate_combinations() {
    let mut source = object();
    source.mesh = source.mesh.with_uv_set(7, vec![[0.5, 0.5]; 3]).unwrap();
    let mut alternate = source.clone();
    alternate.uv_set = 7;
    assert_eq!(
        Memory::plan(&frame(&[source.clone(), alternate.clone()]), C::all())
            .unwrap()
            .meshes,
        1
    );
    let tile = gpui::AtlasTile {
        texture_id: gpui::AtlasTextureId {
            index: 0,
            kind: gpui::AtlasTextureKind::Polychrome,
        },
        tile_id: gpui::TileId(0),
        padding: 0,
        bounds: gpui::Bounds::new(
            gpui::point(gpui::DevicePixels(0), gpui::DevicePixels(0)),
            gpui::size(gpui::DevicePixels(1), gpui::DevicePixels(1)),
        ),
    };
    source.texture = MeshTexture3d::Image(tile);
    alternate.texture = MeshTexture3d::Image(tile);
    let input = frame(&[source, alternate.clone(), alternate]);
    let memory = Memory::plan(&input, C::all()).unwrap();
    let plan = instances::BatchPlan::new(&input, true, 100);
    let mut uploaded = Vec::new();
    let mut cache = geometry::GeometryCache::default();
    cache.prepare(
        plan.order.iter().map(|&index| {
            let object = &input.objects[index];
            (object.mesh.clone(), object.texture_uv_sets())
        }),
        |_, mesh, sets| {
            let vertices: Vec<_> = (0..mesh.vertices().len())
                .map(|index| Vertex::new(mesh, index, sets))
                .collect();
            uploaded.push((
                bytemuck::cast_slice::<_, u8>(&vertices).len(),
                bytemuck::cast_slice::<_, u8>(mesh.indices()).len(),
            ));
        },
    );
    assert_eq!(memory.meshes, uploaded.len() as u64);
    assert_eq!(
        memory.vertex_bytes,
        uploaded
            .iter()
            .map(|(vertices, _)| *vertices as u64)
            .sum::<u64>()
    );
    assert_eq!(
        memory.index_bytes,
        uploaded
            .iter()
            .map(|(_, indices)| *indices as u64)
            .sum::<u64>()
    );
    assert_eq!(memory.total_bytes, 600);
}

#[test]
fn geometry_memory_includes_shadow_only_meshes_and_ignores_fully_culled_meshes() {
    let source = object();
    let mut caster = object();
    caster.model[3][0] = 3.;
    caster.output_id = 42;
    let mut vertices = caster.mesh.vertices().to_vec();
    vertices.push(gpui::MeshVertex3d {
        position: [100., 100., 0.],
        normal: [0., 0., 1.],
        uv: [0.; 2],
    });
    caster.mesh = Mesh3d::new(vertices, vec![0, 1, 2]);
    let mut outside = object();
    outside.model[3][0] = 20.;
    let mut input = frame(&[source, caster, outside]);
    let mut shadow_matrix = IDENTITY;
    shadow_matrix[0][0] = 0.2;
    input.directional_shadow = Some(gpui::DirectionalShadow3d {
        light_index: 0,
        view_projection: shadow_matrix,
        resolution: 256,
        depth_bias: 0.,
        normal_bias: 0.,
        softness: 0.,
    });
    let shaded = Memory::plan(&input, C::all()).unwrap();
    assert!(shaded.validate(383, None).is_err());
    assert!(shaded.validate(384, Some(696)).is_ok());
    assert_eq!(
        (
            shaded.meshes,
            shaded.total_bytes,
            shaded.max_buffer_bytes,
            shaded.max_buffer_object_id
        ),
        (2, 696, 384, Some(42))
    );
    let data = Memory::plan(&input, C::OBJECT_ID | C::LINEAR_DEPTH | C::WORLD_NORMAL).unwrap();
    assert_eq!((data.meshes, data.total_bytes), (1, 300));
    Arc::make_mut(&mut input.objects)[1].cast_shadows = false;
    assert_eq!(Memory::plan(&input, C::COLOR).unwrap(), data);
}

#[test]
fn geometry_memory_checks_vertex_index_and_total_limits_at_exact_boundaries() {
    let mut source = object();
    source.output_id = 18;
    source.mesh = Mesh3d::new(source.mesh.vertices().to_vec(), [0, 1, 2].repeat(100));
    let memory = Memory::plan(&frame(&[source]), C::all()).unwrap();
    assert_eq!(
        (memory.max_buffer_bytes, memory.max_buffer_object_id),
        (1200, Some(18))
    );
    assert!(memory.validate(1200, Some(1488)).is_ok());
    let error = memory.validate(1199, None).unwrap_err().to_string();
    assert!(error.contains("18") && error.contains("1200") && error.contains("1199"));
    assert!(memory.validate(1200, Some(1487)).is_err());
    assert!(memory.validate(u64::MAX, Some(0)).is_err());
    let empty = Memory::plan(&frame(&[]), C::COLOR).unwrap();
    assert_eq!(empty, Memory::default());
    assert!(empty.validate(0, Some(0)).is_ok());
    assert!(Memory::plan(&frame(&[]), C::empty()).is_err());
    assert!(Memory::plan(&frame(&[]), C::from_bits_retain(128)).is_err());
}
