//! Shader programs used by both runtime compilation and the release build.
#![allow(
    dead_code,
    reason = "The renderer and build script use different parts of this module."
)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderSource {
    Inline(&'static str),
    File(&'static str),
}

pub(crate) struct ShaderProgram {
    pub name: &'static str,
    pub source: ShaderSource,
    pub vertex: &'static str,
    pub fragment: &'static str,
}

impl ShaderProgram {
    pub fn entry(&self, target: ShaderTarget) -> &'static str {
        match target {
            ShaderTarget::Vertex => self.vertex,
            ShaderTarget::Fragment => self.fragment,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShaderTarget {
    Vertex,
    Fragment,
}

impl ShaderTarget {
    pub const ALL: [Self; 2] = [Self::Vertex, Self::Fragment];

    pub fn profile(self) -> &'static str {
        match self {
            Self::Vertex => "vs_4_1",
            Self::Fragment => "ps_4_1",
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::Vertex => "vs",
            Self::Fragment => "ps",
        }
    }

    pub fn constant_suffix(self) -> &'static str {
        match self {
            Self::Vertex => "VERTEX_BYTES",
            Self::Fragment => "FRAGMENT_BYTES",
        }
    }
}

macro_rules! shader_programs {
    ($($variant:ident => ($name:literal, $source:expr, $vertex:literal, $fragment:literal)),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub(crate) enum ShaderModule { $($variant),+ }

        impl ShaderModule {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn program(self) -> ShaderProgram {
                match self {
                    $(Self::$variant => ShaderProgram {
                        name: $name,
                        source: $source,
                        vertex: $vertex,
                        fragment: $fragment,
                    }),+
                }
            }

            pub fn uses_raw_instances(self) -> bool {
                self != Self::EmojiRasterization
            }
        }
    };
}

pub(crate) fn compiled_shader_dispatch() -> String {
    let mut source = String::from(
        "fn compiled_shader_bytes(module: ShaderModule, target: ShaderTarget) -> &'static [u8] {\n    match (module, target) {\n",
    );
    for module in ShaderModule::ALL {
        for target in ShaderTarget::ALL {
            source.push_str(&format!(
                "        (ShaderModule::{module:?}, ShaderTarget::{target:?}) => {}_{},\n",
                module.program().name.to_uppercase(),
                target.constant_suffix(),
            ));
        }
    }
    source.push_str("    }\n}\n");
    source
}

use ShaderSource::{File, Inline};
shader_programs! {
    Quad => ("quad", Inline(gpui_render::QUAD_HLSL), "vs_quad", "fs_quad"),
    Shadow => ("shadow", Inline(gpui_render::SHADOW_HLSL), "vs_shadow", "fs_shadow"),
    Underline => ("underline", Inline(gpui_render::UNDERLINE_HLSL), "vs_underline", "fs_underline"),
    PathRasterization => ("path_rasterization", Inline(gpui_render::PATH_RASTERIZATION_HLSL), "vs_path_rasterization", "fs_path_rasterization"),
    PathSprite => ("path_sprite", Inline(gpui_render::PATH_HLSL), "vs_path", "fs_path"),
    MonochromeSprite => ("monochrome_sprite", Inline(gpui_render::MONOCHROME_HLSL), "vs_mono_sprite", "fs_mono_sprite"),
    SubpixelSprite => ("subpixel_sprite", Inline(gpui_render::SUBPIXEL_HLSL), "vs_subpixel_sprite", "fs_subpixel_sprite"),
    PolychromeSprite => ("polychrome_sprite", Inline(gpui_render::POLYCHROME_HLSL), "vs_poly_sprite", "fs_poly_sprite"),
    SurfaceRgba => ("surface_rgba", Inline(gpui_render::SURFACE_HLSL), "vs_surface", "fs_surface_rgba"),
    SurfaceNv12 => ("surface_nv12", Inline(gpui_render::SURFACE_HLSL), "vs_surface", "fs_surface_yuv"),
    EmojiRasterization => ("emoji_rasterization", File("color_text_raster.hlsl"), "emoji_rasterization_vertex", "emoji_rasterization_fragment"),
}
