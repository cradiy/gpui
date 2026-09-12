use naga::{Expression, Function, Handle, Module, ShaderStage, Statement, TypeInner};

fn function(module: &Module, name: &str) -> Handle<Function> {
    module
        .functions
        .iter()
        .find(|(_, function)| function.name.as_deref() == Some(name))
        .unwrap()
        .0
}

#[test]
fn scene3d_material_coverage_is_shared_and_shading_cannot_discard() {
    let module =
        naga::front::wgsl::parse_str(crate::scene3d_material::MaterialProgram::builtin().source())
            .unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let surface = function(&module, "material_surface");
    let coverage = function(&module, "apply_coverage");
    let additional_coverage = function(&module, "apply_mesh_pass_coverage");
    let alpha_coverage = function(&module, "apply_alpha_coverage");
    let shading = function(&module, "material_shading");
    for (handle, function) in module.functions.iter() {
        assert_eq!(
            info[handle].may_kill,
            [coverage, additional_coverage, alpha_coverage].contains(&handle),
            "only renderer coverage may discard: {:?}",
            function.name
        );
    }
    for wrapper in [coverage, additional_coverage] {
        assert_eq!(module.functions[wrapper].body.iter().filter(|statement| {
            matches!(statement, Statement::Call { function, .. } if *function == alpha_coverage)
        }).count(), 1, "coverage wrappers must share alpha interpretation");
    }
    let TypeInner::Vector { size, scalar } =
        module.types[module.functions[shading].result.as_ref().unwrap().ty].inner
    else {
        panic!("shading must return RGB without an alpha override");
    };
    assert_eq!(size, naga::VectorSize::Tri);
    assert_eq!(scalar, naga::Scalar::F32);

    for entry in module
        .entry_points
        .iter()
        .filter(|e| e.stage == ShaderStage::Fragment)
    {
        let coverage = if entry.name == "mesh_pass_fragment" {
            additional_coverage
        } else {
            coverage
        };
        // Each entry must evaluate and clip unconditionally before producing its output.
        let calls: Vec<_> = entry
            .function
            .body
            .iter()
            .filter_map(|statement| {
                if let Statement::Call {
                    function,
                    arguments,
                    result,
                } = statement
                {
                    Some((*function, arguments, *result))
                } else {
                    None
                }
            })
            .collect();
        let surfaces: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.0 == surface)
            .collect();
        let coverages: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.0 == coverage)
            .collect();
        assert_eq!(surfaces.len(), 1, "{} surface evaluation", entry.name);
        assert_eq!(coverages.len(), 1, "{} coverage evaluation", entry.name);
        let (surface_index, surface_call) = surfaces[0];
        let (coverage_index, coverage_call) = coverages[0];
        assert!(surface_index < coverage_index);
        assert_eq!(
            Some(coverage_call.1[0]),
            surface_call.2,
            "{} uses the evaluated surface",
            entry.name
        );
        assert_eq!(
            calls.iter().filter(|call| call.0 == shading).count(),
            usize::from(matches!(
                entry.name.as_str(),
                "fragment" | "mesh_pass_fragment"
            ))
        );
    }
}

#[test]
fn scene3d_camera_and_shadow_preserve_identical_material_vertex_inputs() {
    let module =
        naga::front::wgsl::parse_str(crate::scene3d_material::MaterialProgram::builtin().source())
            .unwrap();
    let world = function(&module, "world_vertex");
    for entry in module
        .entry_points
        .iter()
        .filter(|e| e.stage == ShaderStage::Vertex)
    {
        let calls: Vec<_> = entry
            .function
            .body
            .iter()
            .filter_map(|statement| {
                if let Statement::Call {
                    function,
                    arguments,
                    ..
                } = statement
                {
                    (*function == world).then_some(arguments)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(calls.len(), 1, "{} world attributes", entry.name);
        let arguments = calls[0];
        assert_eq!(arguments.len(), module.functions[world].arguments.len());
        assert_eq!(arguments.len(), entry.function.arguments.len());
        for (index, argument) in arguments.iter().enumerate() {
            assert!(
                matches!(entry.function.expressions[*argument], Expression::FunctionArgument(i) if i as usize == index),
                "{} attribute {} must not be replaced or omitted",
                entry.name,
                index
            );
            assert_eq!(
                entry.function.arguments[index].ty,
                module.functions[world].arguments[index].ty
            );
        }
    }
    let input = module.functions[function(&module, "material_surface")].arguments[0].ty;
    let TypeInner::Struct { members, .. } = &module.types[input].inner else {
        panic!("surface input must be a struct");
    };
    assert_eq!(
        members
            .iter()
            .map(|member| member.name.as_deref().unwrap())
            .collect::<Vec<_>>(),
        [
            "world",
            "normal",
            "tangent",
            "uv",
            "detail_uv",
            "occlusion_uv",
            "color"
        ]
    );
    assert!(members.iter().all(|member| member.binding.is_none()));
}
