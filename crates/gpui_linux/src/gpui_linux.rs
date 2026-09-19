#![cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;

pub use linux::{HeadlessWindowFactory, current_platform, headless_platform_with_window_factory};
#[cfg(feature = "wayland")]
pub use linux::{WaylandSurfaceRoleFactory, wayland_platform_with_external_surface_role};
#[cfg(feature = "wayland")]
pub use wayland_client::Connection as WaylandConnection;
