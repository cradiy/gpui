#[path = "../src/shader_programs.rs"]
mod shader_programs;

use shader_programs::{ShaderModule, ShaderSource, ShaderTarget, compiled_shader_dispatch};
use std::{collections::HashSet, fs, process::Command};

#[test]
fn shader_catalog_matches_generated_entry_points() {
    let wgsl = format!(
        "enable dual_source_blending;\n{}\n{}",
        gpui_render::compose_shader(""),
        gpui_render::SUBPIXEL_WGSL,
    );
    let module = naga::front::wgsl::parse_str(&wgsl).unwrap();
    let mut names = HashSet::new();
    for shader in ShaderModule::ALL {
        let program = shader.program();
        assert!(
            names.insert(program.name),
            "duplicate output: {}",
            program.name
        );
        match program.source {
            ShaderSource::Inline(source) => {
                for target in ShaderTarget::ALL {
                    let entry = program.entry(target);
                    let stage = match target.profile() {
                        "vs_4_1" => naga::ShaderStage::Vertex,
                        "ps_4_1" => naga::ShaderStage::Fragment,
                        profile => panic!("unexpected profile: {profile}"),
                    };
                    assert!(
                        module
                            .entry_points
                            .iter()
                            .any(|e| e.name == entry && e.stage == stage),
                        "missing WGSL entry: {entry}"
                    );
                    assert!(
                        source.contains(&format!(" {entry}(")),
                        "missing HLSL entry: {entry}"
                    );
                }
            }
            ShaderSource::File(file) => {
                let source = fs::read_to_string(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("src")
                        .join(file),
                )
                .unwrap();
                for target in ShaderTarget::ALL {
                    assert!(source.contains(&format!(" {}(", program.entry(target))));
                }
            }
        }
    }
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "Compiles and executes generated Rust dispatch code."
)]
fn release_dispatch_compiles_and_selects_each_stage() {
    let dir = tempfile::tempdir().unwrap();
    let variants = ShaderModule::ALL
        .iter()
        .map(|v| format!("{v:?}"))
        .collect::<Vec<_>>()
        .join(",");
    let mut source =
        format!("enum ShaderModule {{ {variants} }}\nenum ShaderTarget {{ Vertex, Fragment }}\n");
    let mut checks = String::new();
    for (index, module) in ShaderModule::ALL.iter().enumerate() {
        for (stage, target) in ShaderTarget::ALL.iter().enumerate() {
            let value = index * 2 + stage;
            source.push_str(&format!(
                "const {}_{}: &[u8] = &[{value}];\n",
                module.program().name.to_uppercase(),
                target.constant_suffix(),
            ));
            checks.push_str(&format!(
                "assert_eq!(compiled_shader_bytes(ShaderModule::{module:?}, ShaderTarget::{target:?}), &[{value}]);\n",
            ));
        }
    }
    source.push_str(&compiled_shader_dispatch());
    source.push_str(&format!("fn main() {{ {checks} }}"));
    let input = dir.path().join("shader_dispatch.rs");
    let output = dir
        .path()
        .join(format!("shader_dispatch{}", std::env::consts::EXE_SUFFIX));
    fs::write(&input, source).unwrap();
    let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
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
