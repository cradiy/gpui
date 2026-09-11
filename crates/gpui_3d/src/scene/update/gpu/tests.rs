use super::*;
use crate::{
    GpuDeformationOutput, MeshPass, MeshPassExpansion, MeshPassSpace, Scene3dMaterialProgram,
    Scene3dMaterialSource, Scene3dVertexAttribute, Scene3dVertexStreamValue,
};

fn program() -> Result<Scene3dMaterialProgram> {
    Scene3dMaterialProgram::compile_with_attributes(
        r#"
        fn material_surface(input: SurfaceInput, gradients: mat2x2<f32>) -> vec4<f32> {
            return vec4<f32>(input.attributes.width, 0.0, 0.0, 1.0);
        }
        fn material_shading(base: vec3<f32>, input: SurfaceInput,
            gradients: SurfaceGradients, face_sign: f32) -> vec3<f32> { return base; }
    "#,
        &[Scene3dVertexAttribute::new(
            "width",
            wgpu::VertexFormat::Float32,
        )],
    )
}

#[test]
fn submission_width_material_is_valid_wgsl() {
    program().unwrap();
}

#[test]
#[ignore = "requires a compute-capable GPU"]
fn submitted_resources_validate_final_geometry_streams_devices_and_widths() -> Result<()> {
    let context = WgpuContext::new_headless()?;
    let base = Mesh::plane();
    let vertices = GpuDeformationOutput::upload(context.clone(), base.clone(), Default::default())?;
    let packing = vertices.render_source([0; 5], None)?;
    let packed = Arc::new(vertices.render_geometry(&packing)?);
    let program = program()?;
    let material = |context: WgpuContext, count| -> Result<_> {
        let source = Scene3dMaterialSource::new(context, program.clone())?;
        let weights = vec![1_f32; count];
        let streams = source.bind_vertex_streams(
            count,
            &[(
                "width",
                Scene3dVertexStreamValue::Bytes(bytemuck::cast_slice(&weights)),
            )],
            4096,
        )?;
        source
            .bind([], Default::default())?
            .with_vertex_streams(streams)
    };
    let snapshot = material(context.clone(), base.vertex_count())?;
    let input = Scene::new().object(Object::new(
        Mesh::cube(),
        Material::color(gpui::rgb(0xffffff)),
    ));
    let update = ObjectUpdate::new()
        .gpu_geometry(packed.clone(), base.bounds())
        .material(Material::color(gpui::rgb(0xffffff)).program(snapshot.clone()));
    let output = input.with_object_updates([(1, update)])?;
    assert!(Arc::ptr_eq(&output.objects[0].mesh.0, packed.base_mesh()));
    assert_eq!(output.objects[0].render_bounds, Some(base.bounds()));
    assert!(input.objects[0].gpu_geometry.is_none());
    assert!(
        output
            .with_object_updates([(1, ObjectUpdate::new().mesh(Mesh::cube()))])
            .is_err()
    );
    let wrong_count = material(context, base.vertex_count() + 1)?;
    assert!(
        output
            .with_object_updates([(
                1,
                ObjectUpdate::new()
                    .material(Material::color(gpui::rgb(0xffffff)).program(wrong_count))
            )])
            .is_err()
    );
    let foreign = material(WgpuContext::new_headless()?, base.vertex_count())?;
    assert!(
        output
            .with_object_updates([(
                1,
                ObjectUpdate::new().material(Material::color(gpui::rgb(0xffffff)).program(foreign))
            )])
            .is_err()
    );
    let width_pass = |name| {
        Material::color(gpui::rgb(0xffffff)).mesh_passes([MeshPass::new(snapshot.clone())
            .expansion(MeshPassExpansion::new(MeshPassSpace::World, 0.1).weight(name, 1.))])
    };
    output.with_object_updates([(1, ObjectUpdate::new().material(width_pass("width")))])?;
    assert!(
        output
            .with_object_updates([(1, ObjectUpdate::new().material(width_pass("missing")))])
            .is_err()
    );
    let revision = output.preparation_revision.clone();
    let retained = output.with_object_updates([])?;
    assert!(Arc::ptr_eq(&revision, &retained.preparation_revision));
    assert!(Arc::ptr_eq(
        &retained.objects[0]
            .gpu_geometry
            .as_ref()
            .unwrap()
            .downcast::<crate::Scene3dGpuGeometry>()
            .unwrap(),
        &packed
    ));
    Ok(())
}
