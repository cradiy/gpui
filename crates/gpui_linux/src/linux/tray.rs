use calloop::channel::Sender;
use gpui::{Action, Menu, MenuItem, TrayEvent, TrayId, TrayOptions, TrayScrollAxis};
use ksni::menu;

pub(crate) enum LinuxTrayMessage {
    Event(TrayId, TrayEvent),
    Action(Box<dyn Action>),
}

pub(crate) struct GpuiTray {
    id: TrayId,
    service_id: String,
    options: TrayOptions,
    sender: Sender<LinuxTrayMessage>,
}

impl GpuiTray {
    pub(crate) fn new(id: TrayId, options: TrayOptions, sender: Sender<LinuxTrayMessage>) -> Self {
        let executable = std::env::current_exe()
            .ok()
            .and_then(|path| {
                path.file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "gpui".to_string());
        Self {
            id,
            service_id: format!("{executable}-{}", id.as_u32()),
            options,
            sender,
        }
    }

    pub(crate) fn replace_options(&mut self, options: TrayOptions) {
        self.options = options;
    }

    pub(crate) fn spawn(self) -> anyhow::Result<ksni::Handle<Self>> {
        // A desktop's watcher may appear after application autostart. Keep the
        // item alive so ksni can register on watcher arrival and re-register
        // after host restarts, retaining the same handle and latest options.
        smol::block_on(<Self as ksni::TrayMethods>::assume_sni_available(self, true).spawn())
            .map_err(|error| anyhow::anyhow!(error))
    }
}

impl ksni::Tray for GpuiTray {
    fn id(&self) -> String {
        self.service_id.clone()
    }

    fn title(&self) -> String {
        self.options
            .tooltip
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| self.service_id.clone())
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        self.options
            .icon
            .images()
            .iter()
            .map(|image| {
                let mut argb = image.rgba.to_vec();
                for pixel in argb.chunks_exact_mut(4) {
                    pixel.rotate_right(1);
                }
                ksni::Icon {
                    width: image.width as i32,
                    height: image.height as i32,
                    data: argb,
                }
            })
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: self
                .options
                .tooltip
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.sender
            .send(LinuxTrayMessage::Event(self.id, TrayEvent::PrimaryActivate))
            .ok();
        if let Some(action) = &self.options.activate {
            self.sender
                .send(LinuxTrayMessage::Action(action.boxed_clone()))
                .ok();
        }
    }

    fn secondary_activate(&mut self, _x: i32, _y: i32) {
        self.sender
            .send(LinuxTrayMessage::Event(
                self.id,
                TrayEvent::SecondaryActivate,
            ))
            .ok();
    }

    fn scroll(&mut self, delta: i32, orientation: ksni::Orientation) {
        let axis = match orientation {
            ksni::Orientation::Horizontal => TrayScrollAxis::Horizontal,
            ksni::Orientation::Vertical => TrayScrollAxis::Vertical,
        };
        self.sender
            .send(LinuxTrayMessage::Event(
                self.id,
                TrayEvent::Scroll {
                    delta: delta as f32,
                    axis,
                },
            ))
            .ok();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        convert_menu(&self.options.menu, &self.sender)
    }
}

fn convert_menu(
    items: &[MenuItem],
    sender: &Sender<LinuxTrayMessage>,
) -> Vec<ksni::MenuItem<GpuiTray>> {
    items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Separator => Some(ksni::MenuItem::Separator),
            MenuItem::Label(label) => Some(
                menu::StandardItem {
                    label: escape_label(label),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            ),
            MenuItem::Action {
                name,
                action,
                checked,
                checkable,
                disabled,
                ..
            } => {
                let sender = sender.clone();
                let action = action.boxed_clone();
                if *checkable {
                    Some(
                        menu::CheckmarkItem {
                            label: escape_label(name),
                            enabled: !disabled,
                            checked: *checked,
                            activate: Box::new(move |_| {
                                sender
                                    .send(LinuxTrayMessage::Action(action.boxed_clone()))
                                    .ok();
                            }),
                            ..Default::default()
                        }
                        .into(),
                    )
                } else {
                    Some(
                        menu::StandardItem {
                            label: escape_label(name),
                            enabled: !disabled,
                            activate: Box::new(move |_| {
                                sender
                                    .send(LinuxTrayMessage::Action(action.boxed_clone()))
                                    .ok();
                            }),
                            ..Default::default()
                        }
                        .into(),
                    )
                }
            }
            MenuItem::Submenu(Menu {
                name,
                items,
                disabled,
            }) => Some(
                menu::SubMenu {
                    label: escape_label(name),
                    enabled: !disabled,
                    submenu: convert_menu(items, sender),
                    ..Default::default()
                }
                .into(),
            ),
            MenuItem::SystemMenu(_) => None,
        })
        .collect()
}

fn escape_label(label: &str) -> String {
    label.replace('_', "__")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(feature = "wayland", feature = "x11"))]
    #[test]
    #[ignore = "requires a private bus: dbus-run-session -- cargo test -p gpui_linux --lib watcher_lifecycle -- --ignored"]
    fn watcher_lifecycle() {
        use ashpd::zbus;
        use std::time::Duration;

        struct Watcher(smol::channel::Sender<String>);

        #[zbus::interface(name = "org.kde.StatusNotifierWatcher", crate = "ashpd::zbus")]
        impl Watcher {
            fn register_status_notifier_item(&self, service: String) {
                self.0.try_send(service).unwrap();
            }
        }

        async fn start_watcher(sender: smol::channel::Sender<String>) -> zbus::Connection {
            zbus::connection::Builder::session()
                .unwrap()
                .serve_at("/StatusNotifierWatcher", Watcher(sender))
                .unwrap()
                .name("org.kde.StatusNotifierWatcher")
                .unwrap()
                .build()
                .await
                .unwrap()
        }

        smol::block_on(async {
            let bus = zbus::Connection::session().await.unwrap();
            let dbus = zbus::fdo::DBusProxy::new(&bus).await.unwrap();
            assert!(
                !dbus
                    .name_has_owner("org.kde.StatusNotifierWatcher".try_into().unwrap())
                    .await
                    .unwrap(),
                "run this test on a private D-Bus session without a desktop watcher"
            );
            let (sender, _receiver) = calloop::channel::channel();
            let options = TrayOptions::new(
                gpui::TrayIcon::from_images(
                    [gpui::TrayIconImage::new(vec![255; 4], 1, 1).unwrap()],
                )
                .unwrap(),
            );
            let handle = GpuiTray::new(TrayId::from_u32(1), options, sender)
                .spawn()
                .unwrap();
            assert!(
                handle
                    .update(|tray| tray.options.tooltip = Some("Updated offline".into()))
                    .await
                    .is_some()
            );

            let (registered, registrations) = smol::channel::unbounded();
            let mut previous_service = None;
            for _ in 0..2 {
                let watcher = start_watcher(registered.clone()).await;
                let service =
                    smol::future::or(async { registrations.recv().await.unwrap() }, async {
                        smol::Timer::after(Duration::from_secs(5)).await;
                        panic!("tray did not register after watcher arrival");
                    })
                    .await;
                if let Some(previous) = &previous_service {
                    assert_eq!(&service, previous, "host restart must retain the tray item");
                }
                let item = zbus::Proxy::new(
                    &bus,
                    service.as_str(),
                    "/StatusNotifierItem",
                    "org.kde.StatusNotifierItem",
                )
                .await
                .unwrap();
                assert_eq!(
                    item.get_property::<String>("Title").await.unwrap(),
                    "Updated offline"
                );
                previous_service = Some(service.clone());
                watcher.close().await.unwrap();
            }
            // Removing the item must also work while its host is offline.
            handle.shutdown().await;
        });
    }

    #[test]
    fn unchecked_actions_remain_checkboxes_in_native_tray_menu() {
        let (sender, _receiver) = calloop::channel::channel();
        let items = [
            MenuItem::action("Selected", gpui::NoAction).checked(true),
            MenuItem::action("Unselected", gpui::NoAction).checked(false),
            MenuItem::action("Command", gpui::NoAction),
        ];
        let native = convert_menu(&items, &sender);
        assert!(matches!(&native[0], ksni::MenuItem::Checkmark(item) if item.checked));
        assert!(matches!(&native[1], ksni::MenuItem::Checkmark(item) if !item.checked));
        assert!(matches!(&native[2], ksni::MenuItem::Standard(_)));
    }
}
