use super::*;

#[test]
fn status_decoding_distinguishes_vertex_and_triangle_failures() {
    let decode =
        |words: [u32; 8]| Scene3dGeometryStatus::decode(bytemuck::cast_slice(&words), 5, 9);
    let clean = decode([9, 1, 0, 0, 0, 0, u32::MAX, u32::MAX]).unwrap();
    assert!(clean.is_drawable());
    assert_eq!(clean.first_invalid_vertex, None);
    assert_eq!(clean.first_invalid_triangle, None);
    for issue in Scene3dGeometryIssues::all().iter() {
        let triangle = issue == Scene3dGeometryIssues::TRIANGLE_TANGENT_SIGN;
        let status = decode([
            9,
            0,
            0,
            0,
            0,
            issue.bits(),
            if triangle { u32::MAX } else { 4 },
            if triangle { 2 } else { u32::MAX },
        ])
        .unwrap();
        assert!(!status.is_drawable());
        assert_eq!(status.issues, issue);
        assert_eq!(status.first_invalid_vertex, (!triangle).then_some(4));
        assert_eq!(status.first_invalid_triangle, triangle.then_some(2));
    }
    let all = decode([9, 0, 0, 0, 0, Scene3dGeometryIssues::all().bits(), 0, 0]).unwrap();
    assert_eq!(all.first_invalid_vertex, Some(0));
    assert_eq!(all.first_invalid_triangle, Some(0));
    for invalid in [
        [9, 1, 0, 0, 0, 1, 0, u32::MAX],
        [9, 0, 0, 0, 0, 0, u32::MAX, u32::MAX],
        [9, 0, 0, 0, 0, 64, 0, 0],
        [9, 0, 0, 0, 0, 1, 5, u32::MAX],
        [9, 0, 0, 0, 0, 32, u32::MAX, 3],
        [9, 0, 0, 0, 0, 1, u32::MAX, u32::MAX],
        [9, 0, 0, 0, 0, 32, 0, u32::MAX],
        [6, 1, 0, 0, 0, 0, u32::MAX, u32::MAX],
        [9, 1, 1, 0, 0, 0, u32::MAX, u32::MAX],
    ] {
        assert!(decode(invalid).is_err(), "{invalid:?}");
    }
    assert!(Scene3dGeometryStatus::decode(&[0; 31], 5, 9).is_err());
}

#[test]
fn geometry_shader_issue_bits_match_public_status_flags() {
    use wgpu::naga::{Expression, Literal};
    let module =
        wgpu::naga::front::wgsl::parse_str(include_str!("../../gpu_geometry.wgsl")).unwrap();
    for (name, flag) in Scene3dGeometryIssues::all().iter_names() {
        let (_, constant) = module
            .constants
            .iter()
            .find(|(_, c)| c.name.as_deref() == Some(name))
            .unwrap();
        let Expression::Literal(Literal::U32(value)) = module.global_expressions[constant.init]
        else {
            panic!("expected status bit literal");
        };
        assert_eq!(value, flag.bits(), "{name}");
    }
}
