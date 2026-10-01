use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=src/common.wgsl");
    for primitive in [
        "quads",
        "shadows",
        "underlines",
        "path_rasterization",
        "paths",
        "polychrome_sprites",
    ] {
        println!("cargo:rerun-if-changed=src/{primitive}.wgsl");
        generate(primitive);
    }
}

fn generate(primitive: &str) {
    let source = format!(
        "{}\n{}",
        fs::read_to_string("src/common.wgsl").unwrap(),
        fs::read_to_string(format!("src/{primitive}.wgsl")).unwrap()
    );
    assert_eq!(
        source.matches("const NATIVE_SRGB: bool = false;").count(),
        1
    );
    let source = source.replace(
        "const NATIVE_SRGB: bool = false;",
        "const NATIVE_SRGB: bool = true;",
    );
    let mut module = naga::front::wgsl::parse_str(&source)
        .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    naga::compact::compact(&mut module, naga::compact::KeepUnused::No);
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut msl = naga::back::msl::Options {
        lang_version: (2, 0),
        fake_missing_bindings: false,
        ..Default::default()
    };
    let mut resources = naga::back::msl::EntryPointResources {
        sizes_buffer: Some(3),
        ..Default::default()
    };
    let mut hlsl = naga::back::hlsl::Options {
        shader_model: naga::back::hlsl::ShaderModel::V5_0,
        fake_missing_bindings: false,
        ..Default::default()
    };
    hlsl.sampler_buffer_binding_map.insert(
        naga::back::hlsl::SamplerIndexBufferKey { group: 1 },
        naga::back::hlsl::BindTarget {
            space: 0,
            register: 2,
            ..Default::default()
        },
    );
    for (_, variable) in module.global_variables.iter() {
        let Some(binding) = variable.binding else {
            continue;
        };
        let (slot, target) = match (binding.group, binding.binding, variable.space) {
            (0, 0, naga::AddressSpace::Uniform) => (
                0,
                naga::back::msl::BindTarget {
                    buffer: Some(0),
                    ..Default::default()
                },
            ),
            (1, 0, naga::AddressSpace::Storage { .. }) => (
                1,
                naga::back::msl::BindTarget {
                    buffer: Some(1),
                    ..Default::default()
                },
            ),
            (1, 1, naga::AddressSpace::Handle) => (
                0,
                naga::back::msl::BindTarget {
                    texture: Some(0),
                    ..Default::default()
                },
            ),
            (1, 2, naga::AddressSpace::Handle) => (
                0,
                naga::back::msl::BindTarget {
                    sampler: Some(naga::back::msl::BindSamplerTarget::Resource(0)),
                    ..Default::default()
                },
            ),
            _ => panic!("unexpected {primitive} shader resource: {binding:?}"),
        };
        resources.resources.insert(binding, target);
        hlsl.binding_map.insert(
            binding,
            naga::back::hlsl::BindTarget {
                space: 0,
                register: slot as u32,
                ..Default::default()
            },
        );
    }
    for entry in &module.entry_points {
        msl.per_entry_point_map
            .insert(entry.name.clone(), resources.clone());
    }
    let (source, reflection) =
        naga::back::msl::write_string(&module, &info, &msl, &Default::default()).unwrap();
    for (entry, name) in module.entry_points.iter().zip(reflection.entry_point_names) {
        assert_eq!(entry.name, name.unwrap(), "native entry point was renamed");
    }
    fs::write(out.join(format!("{primitive}.metal")), source).unwrap();
    let mut source = String::new();
    naga::back::hlsl::Writer::new(&mut source, &hlsl, &Default::default())
        .write(&module, &info, None)
        .unwrap();
    if module.global_variables.iter().any(|(_, variable)| {
        matches!(
            module.types[variable.ty].inner,
            naga::TypeInner::Sampler { .. }
        )
    }) {
        source = directx11_sprite_sampler(source);
    }
    fs::write(out.join(format!("{primitive}.hlsl")), source).unwrap();
}

// Naga 30 emits D3D12 sampler heaps. Shared sprites have one statically bound
// sampler, so D3D11 can use the same sample operations with a direct s0 binding.
// Reject a changed Naga interface rather than emitting an incompatible shader.
fn directx11_sprite_sampler(source: String) -> String {
    const HEAP: &str = concat!(
        "SamplerState nagaSamplerHeap[2048]: register(s0, space0);\n",
        "SamplerComparisonState nagaComparisonSamplerHeap[2048]: register(s0, space1);\n",
        "StructuredBuffer<uint> nagaGroup1SamplerIndexArray : register(t2, space0);\n",
        "static const SamplerState s_sprite = nagaSamplerHeap[nagaGroup1SamplerIndexArray[0]];\n",
    );
    assert_eq!(
        source.matches(HEAP).count(),
        1,
        "Naga static sampler interface changed"
    );
    let source = source.replace(HEAP, "SamplerState s_sprite : register(s0);\n");
    assert!(
        !source.contains("SamplerHeap") && !source.contains("SamplerIndexArray"),
        "unsupported dynamic sampler access"
    );
    source
}
