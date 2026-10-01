#![allow(clippy::disallowed_methods, reason = "build scripts are exempt")]

fn main() {
    println!("cargo:rerun-if-changed=src/shaders.metal");
    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-link-lib=framework=MetalPerformanceShaders");
        #[cfg(not(feature = "runtime_shaders"))]
        macos_build::compile_shaders();
    }
}

#[cfg(all(target_os = "macos", not(feature = "runtime_shaders")))]
mod macos_build {
    use std::{
        env,
        path::{Path, PathBuf},
        process::Command,
    };

    pub fn compile_shaders() {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        compile_shader("shaders", Path::new("src/shaders.metal"), &out);
        for (name, shader) in [
            ("quads", gpui_render::QUAD_MSL),
            ("shadows", gpui_render::SHADOW_MSL),
            ("underlines", gpui_render::UNDERLINE_MSL),
            ("path_rasterization", gpui_render::PATH_RASTERIZATION_MSL),
            ("paths", gpui_render::PATH_MSL),
            ("polychrome_sprites", gpui_render::POLYCHROME_MSL),
            ("monochrome_sprites", gpui_render::MONOCHROME_MSL),
            ("surfaces", gpui_render::SURFACE_MSL),
        ] {
            let source = out.join(format!("{name}.metal"));
            std::fs::write(&source, shader).unwrap();
            compile_shader(name, &source, &out);
        }
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
