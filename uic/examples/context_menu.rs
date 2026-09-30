use gpui::{Context, Entity, IntoElement, Render, Window, WindowOptions, div, prelude::*, px, rgb};
use uic::components::context_menu::{self, ContextMenu, ContextMenuExt};

#[path = "support/context_menu_style.rs"]
mod context_menu_style;
use context_menu_style::MenuMaterial;

struct ContextMenuExample {
    material: MenuMaterial,
    last_action: &'static str,
}

impl ContextMenuExample {
    fn set_material(
        entity: &Entity<Self>,
        material: MenuMaterial,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        entity.update(cx, |this, cx| {
            this.material = material;
            this.last_action = material.label();
            cx.notify();
        });
        window.refresh();
    }

    fn menu(entity: Entity<Self>, cx: &gpui::App) -> ContextMenu {
        let material = entity.read(cx).material;
        let dark_entity = entity.clone();
        let light_entity = entity.clone();
        let plain_entity = entity.clone();
        let rename_entity = entity.clone();
        let archive_entity = entity.clone();

        context_menu_style::menu(material)
            .action_with_shortcut("Open", "Enter", |_, _| {})
            .submenu("Material", move |menu| {
                menu.action("Dark frosted", move |window, cx| {
                    Self::set_material(&dark_entity, MenuMaterial::DarkFrosted, window, cx);
                })
                .action("Light frosted", move |window, cx| {
                    Self::set_material(&light_entity, MenuMaterial::LightFrosted, window, cx);
                })
                .submenu("More", move |menu| {
                    menu.action("Plain div", move |window, cx| {
                        Self::set_material(&plain_entity, MenuMaterial::Plain, window, cx);
                    })
                    .action("Material settings…", |_, _| {})
                })
            })
            .separator()
            .action("Rename", move |_, cx| {
                rename_entity.update(cx, |this, cx| {
                    this.last_action = "Rename";
                    cx.notify();
                });
            })
            .item(
                uic::components::context_menu::ContextMenuItem::action("Archive", move |_, cx| {
                    archive_entity.update(cx, |this, cx| {
                        this.last_action = "Archive";
                        cx.notify();
                    });
                })
                .danger(),
            )
    }
}

impl Render for ContextMenuExample {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let menu_entity = entity.clone();
        let context_menu_layer = context_menu::layer(cx);

        div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(0x0f172a))
            .text_color(rgb(0xf8fafc))
            .child(
                div()
                    .w(px(440.))
                    .p_8()
                    .rounded(px(24.))
                    .border_1()
                    .border_color(rgb(0x334155))
                    .bg(rgb(0x172033))
                    .flex()
                    .flex_col()
                    .gap_3()
                    .context_menu(move |_, cx| Self::menu(menu_entity.clone(), cx))
                    .child(div().text_xl().child("Context menu surfaces"))
                    .child("Right-click this card. The menu supports three levels.")
                    .child(format!("Root material: {}", self.material.label()))
                    .child(format!("Last action: {}", self.last_action)),
            )
            // The layer must be the last child so all menu levels paint above application content.
            .child(context_menu_layer)
    }
}

fn main() {
    gpui_platform::application().run(|cx| {
        uic::init(cx);
        cx.open_window(WindowOptions::default(), |_, cx| {
            cx.new(|_| ContextMenuExample {
                material: MenuMaterial::DarkFrosted,
                last_action: "None",
            })
        })
        .expect("failed to open context menu example window");
    });
}
