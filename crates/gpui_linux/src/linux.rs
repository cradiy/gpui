mod dispatcher;
mod headless;
mod keyboard;
mod platform;
#[cfg(any(feature = "wayland", feature = "x11"))]
mod text_system;
mod tray;
#[cfg(feature = "wayland")]
mod wayland;
#[cfg(feature = "x11")]
mod x11;

#[cfg(any(feature = "wayland", feature = "x11"))]
mod xdg_desktop_portal;

pub use dispatcher::*;
pub use headless::HeadlessWindowFactory;
pub(crate) use headless::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
#[cfg(any(feature = "wayland", feature = "x11"))]
pub(crate) use text_system::*;
pub(crate) use tray::*;
#[cfg(feature = "wayland")]
pub(crate) use wayland::*;
#[cfg(feature = "x11")]
pub(crate) use x11::*;

use std::rc::Rc;

#[cfg(feature = "wayland")]
pub type WaylandSurfaceRoleFactory = wayland::ExternalWaylandSurfaceRoleFactory;

/// Returns the default platform implementation for the current OS.
pub fn current_platform(headless: bool) -> Rc<dyn gpui::Platform> {
    #[cfg(feature = "x11")]
    use anyhow::Context as _;

    if headless {
        return Rc::new(LinuxPlatform {
            inner: HeadlessClient::new(),
        });
    }

    match gpui::guess_compositor() {
        #[cfg(feature = "wayland")]
        "Wayland" => Rc::new(LinuxPlatform {
            inner: WaylandClient::new(),
        }),

        #[cfg(feature = "x11")]
        "X11" => Rc::new(LinuxPlatform {
            inner: X11Client::new()
                .context("Failed to initialize X11 client.")
                .unwrap(),
        }),

        "Headless" => Rc::new(LinuxPlatform {
            inner: HeadlessClient::new(),
        }),
        _ => unreachable!(
            r#"At least one of the "wayland" or "x11" features must be enabled on gpui_linux or gpui_platform."#
        ),
    }
}

/// Returns a Wayland platform that reuses an existing client connection and
/// assigns a caller-provided role to every window surface it creates.
#[cfg(feature = "wayland")]
pub fn wayland_platform_with_external_surface_role(
    connection: wayland_client::Connection,
    role: WaylandSurfaceRoleFactory,
) -> Rc<dyn gpui::Platform> {
    Rc::new(LinuxPlatform {
        inner: WaylandClient::with_connection_and_external_surface_role(connection, role),
    })
}

/// Returns a headless platform whose windows are supplied by an external host.
pub fn headless_platform_with_window_factory(
    factory: Rc<dyn HeadlessWindowFactory>,
) -> Rc<dyn gpui::Platform> {
    Rc::new(LinuxPlatform {
        inner: HeadlessClient::with_window_factory(Some(factory)),
    })
}
