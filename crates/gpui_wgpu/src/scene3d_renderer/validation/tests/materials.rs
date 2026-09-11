use super::*;
use crate::wgpu_renderer::scene3d::tests::object;
use gpui::{MeshMaterial3d, MeshPass3d};
use std::sync::Arc;

#[test]
fn material_admission_rejects_unsupported_primary_and_additional_backends() {
    for additional in [false, true] {
        let mut object = object();
        object.model[3][0] = 1000.;
        let material = MeshMaterial3d::new(Arc::new(()));
        if additional {
            object.mesh_passes = vec![MeshPass3d {
                material,
                state: Default::default(),
                expansion: None,
            }]
            .into();
        } else {
            object.custom_material = Some(material);
        }
        for shaded in [false, true] {
            let error =
                validate_frame_settings(&frame(&[object.clone()]), 1024, shaded).unwrap_err();
            assert!(error.to_string().contains("backend"), "{error}");
        }
    }
}

#[test]
fn mesh_pass_admission_checks_limits_and_state_before_backend_resources() {
    let pass = MeshPass3d {
        material: MeshMaterial3d::new(Arc::new(())),
        state: Default::default(),
        expansion: None,
    };
    let mut object = object();
    object.mesh_passes = vec![pass.clone(); gpui::MAX_MESH_PASSES_3D + 1].into();
    let error = validate_frame_settings(&frame(&[object.clone()]), 1024, true).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("too many additional mesh passes"),
        "{error}"
    );

    for invalid in [f32::NAN, f32::INFINITY, -1.] {
        let mut invalid_pass = pass.clone();
        invalid_pass.state.alpha_cutoff = invalid;
        object.mesh_passes = vec![invalid_pass].into();
        for shaded in [false, true] {
            let error =
                validate_frame_settings(&frame(&[object.clone()]), 1024, shaded).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("invalid additional mesh pass state"),
                "{error}"
            );
        }
    }
}
