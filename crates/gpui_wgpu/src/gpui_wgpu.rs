mod cosmic_text_system;
mod wgpu_atlas;
mod wgpu_context;
mod wgpu_renderer;

pub use cosmic_text_system::*;
pub use wgpu;
pub use wgpu_atlas::*;
pub use wgpu_context::*;
pub use wgpu_renderer::{GpuContext, WgpuRenderer, WgpuSurfaceConfig};

#[cfg(test)]
mod tests {
    fn shader_struct_span(module: &wgpu::naga::Module, name: &str) -> u32 {
        module
            .types
            .iter()
            .find_map(|(_, ty)| {
                if ty.name.as_deref() != Some(name) {
                    return None;
                }
                match &ty.inner {
                    wgpu::naga::TypeInner::Struct { span, .. } => Some(*span),
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("missing shader struct {name}"))
    }

    fn shader_struct_offsets(module: &wgpu::naga::Module, name: &str) -> Vec<(String, u32)> {
        module
            .types
            .iter()
            .find_map(|(_, ty)| {
                if ty.name.as_deref() != Some(name) {
                    return None;
                }
                match &ty.inner {
                    wgpu::naga::TypeInner::Struct { members, .. } => Some(
                        members
                            .iter()
                            .map(|member| {
                                (
                                    member.name.as_deref().unwrap_or("?").to_owned(),
                                    member.offset,
                                )
                            })
                            .collect(),
                    ),
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("missing shader struct {name}"))
    }

    #[test]
    fn main_shader_is_valid_wgsl() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shaders.wgsl"))
            .expect("main shader should parse");
        wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("main shader should validate");
    }

    #[test]
    fn shader_struct_layouts_match_rust() {
        let module = wgpu::naga::front::wgsl::parse_str(include_str!("shaders.wgsl"))
            .expect("main shader should parse");

        assert_eq!(
            shader_struct_span(&module, "Background") as usize,
            std::mem::size_of::<gpui::Background>()
        );
        assert_eq!(
            shader_struct_offsets(&module, "Quad"),
            vec![
                (
                    "order".into(),
                    std::mem::offset_of!(gpui::Quad, order) as u32
                ),
                (
                    "border_style".into(),
                    std::mem::offset_of!(gpui::Quad, border_style) as u32,
                ),
                (
                    "bounds".into(),
                    std::mem::offset_of!(gpui::Quad, bounds) as u32
                ),
                (
                    "content_mask".into(),
                    std::mem::offset_of!(gpui::Quad, content_mask) as u32,
                ),
                (
                    "background".into(),
                    std::mem::offset_of!(gpui::Quad, background) as u32,
                ),
                (
                    "border_colors".into(),
                    std::mem::offset_of!(gpui::Quad, border_colors) as u32,
                ),
                (
                    "border_gradient".into(),
                    std::mem::offset_of!(gpui::Quad, border_gradient) as u32,
                ),
                (
                    "corner_radii".into(),
                    std::mem::offset_of!(gpui::Quad, corner_radii) as u32,
                ),
                (
                    "border_widths".into(),
                    std::mem::offset_of!(gpui::Quad, border_widths) as u32,
                ),
            ]
        );
        assert_eq!(
            shader_struct_span(&module, "Quad") as usize,
            std::mem::size_of::<gpui::Quad>()
        );
        assert_eq!(
            shader_struct_span(&module, "PathRasterizationVertex") as usize,
            std::mem::size_of::<super::wgpu_renderer::PathRasterizationVertex>()
        );
    }
}
