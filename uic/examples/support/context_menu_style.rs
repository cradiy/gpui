use gpui::{IntoElement, div, prelude::*, px, rgb, rgba, transparent_black};
use gpui_effects::{FrostedGlass, FrostedGlassAppearance};
use uic::components::context_menu::{ContextMenu, ContextMenuAppearance};

#[derive(Clone, Copy)]
pub enum MenuMaterial {
    DarkFrosted,
    LightFrosted,
    Plain,
}

impl MenuMaterial {
    pub fn label(self) -> &'static str {
        match self {
            Self::DarkFrosted => "Dark frosted",
            Self::LightFrosted => "Light frosted",
            Self::Plain => "Plain div",
        }
    }
}

pub fn menu(material: MenuMaterial) -> ContextMenu {
    ContextMenu::new()
        .appearance(
            ContextMenuAppearance::default()
                .muted_foreground(rgb(0x94a3b8).into())
                .danger_foreground(rgb(0xfb7185).into())
                .selected_background(rgba(0x94a3b829).into())
                .selected_foreground(rgb(0xffffff).into())
                .item_height(px(34.0))
                .item_padding_x(px(10.0))
                .item_radius(px(8.0))
                .separator(rgb(0x64748b).into())
                .separator_margin(px(5.0)),
        )
        .w(px(220.0))
        .max_h(px(420.0))
        .p(px(8.0))
        .rounded(px(16.0))
        .border(px(0.0))
        .bg(transparent_black())
        .text_color(rgb(0xf8fafc))
        .font_family(".SystemUIFont")
        .root_surface(move |state, content, _, _| match material {
            MenuMaterial::DarkFrosted => {
                FrostedGlass::with_appearance(FrostedGlassAppearance::dark())
                    .id(("context-menu-dark-frosted", state.session_id))
                    .rounded(px(16.))
                    .shadow_lg()
                    .child(content)
                    .into_any_element()
            }
            MenuMaterial::LightFrosted => {
                FrostedGlass::with_appearance(FrostedGlassAppearance::light())
                    .id(("context-menu-light-frosted", state.session_id))
                    .rounded(px(14.))
                    .shadow_lg()
                    .child(content)
                    .into_any_element()
            }
            MenuMaterial::Plain => div()
                .rounded(px(12.))
                .border_1()
                .border_color(rgb(0x334155))
                .bg(rgb(0x111827))
                .shadow_lg()
                .child(content)
                .into_any_element(),
        })
        .submenu_surface(|state, content, _, _| {
            FrostedGlass::with_appearance(FrostedGlassAppearance::dark())
                .id((
                    "context-submenu-frosted",
                    state.session_id * 10 + state.depth as u64,
                ))
                .rounded(px(13.))
                .shadow_lg()
                .child(content)
        })
        .surface_for_depth(2, |_, content, _, _| {
            div()
                .rounded(px(11.))
                .border_1()
                .border_color(rgb(0x475569))
                .bg(rgb(0x1e294b))
                .shadow_lg()
                .child(content)
        })
}
