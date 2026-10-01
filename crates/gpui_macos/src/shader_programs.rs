//! Metal libraries and entry points shared by the renderer and build script.
#![allow(
    dead_code,
    reason = "The renderer, build script and contract tests use different parts of this module."
)]

pub(crate) struct Program {
    pub library: ShaderLibrary,
    pub label: &'static str,
    pub vertex: &'static str,
    pub fragment: &'static str,
}

macro_rules! shader_libraries {
    ($($library:ident => ($name:literal, $source:expr, {
        $($program:ident => ($label:literal, $vertex:literal, $fragment:literal)),+ $(,)?
    })),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub(crate) enum ShaderLibrary { $($library),+ }

        impl ShaderLibrary {
            pub const ALL: &'static [Self] = &[$(Self::$library),+];

            pub fn name(self) -> &'static str {
                match self { $(Self::$library => $name),+ }
            }

            pub fn source(self) -> &'static str {
                match self { $(Self::$library => $source),+ }
            }
        }

        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum ShaderProgram { $($($program),+),+ }

        impl ShaderProgram {
            pub const ALL: &'static [Self] = &[$($(Self::$program),+),+];

            pub fn program(self) -> Program {
                match self {
                    $($(Self::$program => Program {
                        library: ShaderLibrary::$library,
                        label: $label,
                        vertex: $vertex,
                        fragment: $fragment,
                    }),+),+
                }
            }
        }
    };
}

shader_libraries! {
    Quad => ("quads", gpui_render::QUAD_MSL, {
        Quad => ("quads", "vs_quad", "fs_quad"),
    }),
    Shadow => ("shadows", gpui_render::SHADOW_MSL, {
        Shadow => ("shadows", "vs_shadow", "fs_shadow"),
    }),
    Underline => ("underlines", gpui_render::UNDERLINE_MSL, {
        Underline => ("underlines", "vs_underline", "fs_underline"),
    }),
    PathRasterization => ("path_rasterization", gpui_render::PATH_RASTERIZATION_MSL, {
        PathRasterization => ("paths_rasterization", "vs_path_rasterization", "fs_path_rasterization"),
    }),
    PathSprite => ("paths", gpui_render::PATH_MSL, {
        PathSprite => ("path_sprites", "vs_path", "fs_path"),
    }),
    PolychromeSprite => ("polychrome_sprites", gpui_render::POLYCHROME_MSL, {
        PolychromeSprite => ("polychrome_sprites", "vs_poly_sprite", "fs_poly_sprite"),
    }),
    MonochromeSprite => ("monochrome_sprites", gpui_render::MONOCHROME_MSL, {
        MonochromeSprite => ("monochrome_sprites", "vs_mono_sprite", "fs_mono_sprite"),
    }),
    Surface => ("surfaces", gpui_render::SURFACE_MSL, {
        SurfaceRgba => ("surfaces_rgba", "vs_surface", "fs_surface_rgba"),
        SurfaceNv12 => ("surfaces_nv12", "vs_surface", "fs_surface_yuv"),
    }),
    Subtree => ("shaders", include_str!("shaders.metal"), {
        Subtree => ("gpui.subtree_composite", "subtree_vertex", "subtree_fragment"),
    }),
}

pub(crate) fn compiled_library_dispatch() -> String {
    let mut source = String::from(
        "fn compiled_library_bytes(library: ShaderLibrary) -> &'static [u8] {\n    match library {\n",
    );
    for library in ShaderLibrary::ALL {
        source.push_str(&format!(
            "        ShaderLibrary::{library:?} => include_bytes!(concat!(env!(\"OUT_DIR\"), \"/{}.metallib\")),\n",
            library.name(),
        ));
    }
    source.push_str("    }\n}\n");
    source
}
