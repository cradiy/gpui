use super::*;
use std::mem::{offset_of, size_of};

#[test]
fn quad_shader_matches_host_layout_and_resource_contract() {
    let module = naga::front::wgsl::parse_str(QUAD_WGSL).unwrap();
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    for (name, size, offsets) in [
        (
            "GlobalParams",
            size_of::<QuadGlobals>(),
            vec![
                offset_of!(QuadGlobals, viewport_size),
                offset_of!(QuadGlobals, premultiplied_alpha),
                offset_of!(QuadGlobals, pad),
                offset_of!(QuadGlobals, viewport_origin),
                offset_of!(QuadGlobals, origin_pad),
            ],
        ),
        (
            "Quad",
            size_of::<gpui::Quad>(),
            vec![
                offset_of!(gpui::Quad, order),
                offset_of!(gpui::Quad, border_style),
                offset_of!(gpui::Quad, bounds),
                offset_of!(gpui::Quad, content_mask),
                offset_of!(gpui::Quad, background),
                offset_of!(gpui::Quad, border_colors),
                offset_of!(gpui::Quad, border_gradient),
                offset_of!(gpui::Quad, corner_radii),
                offset_of!(gpui::Quad, border_widths),
            ],
        ),
    ] {
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap();
        let naga::TypeInner::Struct { members, span } = &ty.inner else {
            panic!("not a struct")
        };
        assert_eq!(*span as usize, size, "{name} size");
        assert_eq!(
            members
                .iter()
                .map(|member| member.offset as usize)
                .collect::<Vec<_>>(),
            offsets,
            "{name} offsets"
        );
    }
    for (name, size) in [
        ("Background", size_of::<gpui::Background>()),
        ("BorderGradient", size_of::<gpui::BorderGradient>()),
    ] {
        let (_, ty) = module
            .types
            .iter()
            .find(|(_, ty)| ty.name.as_deref() == Some(name))
            .unwrap();
        let naga::TypeInner::Struct { span, .. } = ty.inner else {
            panic!("not a struct")
        };
        assert_eq!(span as usize, size, "{name}");
    }
    for (name, group, binding) in [("globals", 0, 0), ("b_quads", 1, 0)] {
        let (_, variable) = module
            .global_variables
            .iter()
            .find(|(_, variable)| variable.name.as_deref() == Some(name))
            .unwrap();
        assert_eq!(
            variable.binding,
            Some(naga::ResourceBinding { group, binding })
        );
    }
}
