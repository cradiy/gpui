use std::any::Any;
use std::rc::Rc;

use wayland_client::protocol::wl_surface::WlSurface;

/// Assigns a compositor-specific role to a GPUI-created Wayland surface.
///
/// The returned handle is retained for exactly as long as the surface role.
/// Dropping it must destroy the protocol role before GPUI destroys the
/// underlying `wl_surface`.
#[derive(Clone)]
pub struct ExternalWaylandSurfaceRoleFactory {
    assign: Rc<dyn Fn(&WlSurface) -> anyhow::Result<Box<dyn Any>>>,
}

impl ExternalWaylandSurfaceRoleFactory {
    pub fn new(assign: impl Fn(&WlSurface) -> anyhow::Result<Box<dyn Any>> + 'static) -> Self {
        Self {
            assign: Rc::new(assign),
        }
    }

    pub(crate) fn assign(&self, surface: &WlSurface) -> anyhow::Result<Box<dyn Any>> {
        (self.assign)(surface)
    }
}
