#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

#[cfg(all(target_os = "macos", not(feature = "runtime_shaders")))]
#[path = "src/shader_programs.rs"]
mod shader_programs;

fn main() {
    println!("cargo:rerun-if-changed=src/shaders.metal");
    println!("cargo:rerun-if-changed=src/shader_programs.rs");
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
        #[cfg(not(feature = "runtime_shaders"))]
        macos_build::compile_shaders();
    }
}

#[cfg(all(target_os = "macos", not(feature = "runtime_shaders")))]
mod macos_build {
    use crate::shader_programs::{ShaderLibrary, compiled_library_dispatch};
    use std::{
        env,
        path::{Path, PathBuf},
        process::Command,
    };

    pub fn compile_shaders() {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        for library in ShaderLibrary::ALL {
            let name = library.name();
            let source = out.join(format!("{name}.metal"));
            std::fs::write(&source, library.source()).unwrap();
            compile_shader(name, &source, &out);
        }
        std::fs::write(out.join("shader_libraries.rs"), compiled_library_dispatch()).unwrap();
    }

    fn compile_shader(name: &str, source: &Path, out: &Path) {
        let air = out.join(format!("{name}.air"));
        let output = Command::new("xcrun")
            .args([
                "-sdk",
                "macosx",
                "metal",
                "-gline-tables-only",
                "-mmacosx-version-min=10.15.7",
                "-c",
            ])
            .arg(source)
            .arg("-o")
            .arg(&air)
            .output()
            .expect("failed to run the Metal compiler");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = Command::new("xcrun")
            .args(["-sdk", "macosx", "metallib"])
            .arg(air)
            .arg("-o")
            .arg(out.join(format!("{name}.metallib")))
            .output()
            .expect("failed to run the Metal library linker");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
