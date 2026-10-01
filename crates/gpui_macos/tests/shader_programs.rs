#[path = "../src/shader_programs.rs"]
mod shader_programs;

use shader_programs::{ShaderLibrary, ShaderProgram, compiled_library_dispatch};
use std::{collections::HashSet, fs, process::Command};

#[test]
fn library_catalog_matches_shader_entry_points() {
    let module = naga::front::wgsl::parse_str(&gpui_render::compose_shader("")).unwrap();
    let mut names = HashSet::new();
    for library in ShaderLibrary::ALL {
        assert!(names.insert(library.name()), "duplicate library output");
        assert!(
            ShaderProgram::ALL
                .iter()
                .any(|p| p.program().library == *library)
        );
    }
    for shader in ShaderProgram::ALL {
        let program = shader.program();
        let source = program.library.source();
        for (entry, stage, qualifier) in [
            (program.vertex, naga::ShaderStage::Vertex, "vertex "),
            (program.fragment, naga::ShaderStage::Fragment, "fragment "),
        ] {
            assert!(
                source.lines().any(|line| line.starts_with(qualifier)
                    && line.contains(&format!(" {entry}("))),
                "missing Metal {stage:?} entry {entry} in {}",
                program.library.name(),
            );
            if program.library != ShaderLibrary::Subtree {
                assert!(
                    module
                        .entry_points
                        .iter()
                        .any(|e| e.name == entry && e.stage == stage),
                    "missing WGSL entry {entry}"
                );
            }
        }
    }
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "Compiles and executes generated Rust library dispatch code."
)]
fn compiled_library_dispatch_loads_each_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let variants = ShaderLibrary::ALL
        .iter()
        .map(|v| format!("{v:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut source = format!("enum ShaderLibrary {{ {variants} }}\n");
    source.push_str(&compiled_library_dispatch());
    source.push_str("fn main() {\n");
    for (index, library) in ShaderLibrary::ALL.iter().enumerate() {
        fs::write(
            dir.path().join(format!("{}.metallib", library.name())),
            [index as u8],
        )
        .unwrap();
        source.push_str(&format!(
            "assert_eq!(compiled_library_bytes(ShaderLibrary::{library:?}), &[{index}]);\n"
        ));
    }
    source.push_str("}\n");
    let input = dir.path().join("library_dispatch.rs");
    let output = dir
        .path()
        .join(format!("library_dispatch{}", std::env::consts::EXE_SUFFIX));
    fs::write(&input, source).unwrap();
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
        .env("OUT_DIR", dir.path())
        .arg("--edition=2024")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = Command::new(output).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[cfg(all(target_os = "macos", not(feature = "runtime_shaders")))]
include!(concat!(env!("OUT_DIR"), "/shader_libraries.rs"));

#[test]
#[cfg(target_os = "macos")]
fn metal_libraries_expose_catalogued_functions() {
    let device = metal::Device::system_default().expect("Metal device is required");
    for &shader in ShaderLibrary::ALL {
        #[cfg(feature = "runtime_shaders")]
        let library =
            device.new_library_with_source(shader.source(), &metal::CompileOptions::new());
        #[cfg(not(feature = "runtime_shaders"))]
        let library = device.new_library_with_data(compiled_library_bytes(shader));
        let library = library.unwrap_or_else(|error| panic!("{}: {error}", shader.name()));
        for program in ShaderProgram::ALL
            .iter()
            .map(|p| p.program())
            .filter(|p| p.library == shader)
        {
            for entry in [program.vertex, program.fragment] {
                library
                    .get_function(entry, None)
                    .unwrap_or_else(|error| panic!("{entry}: {error}"));
            }
        }
    }
}
