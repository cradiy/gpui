use super::*;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};

pub(crate) struct X11DragIcon {
    connection: Rc<XCBConnection>,
    pub(crate) window: u32,
    colormap: u32,
    renderer: Option<WgpuRenderer>,
    hotspot: Point<Pixels>,
    scale: f32,
}

impl X11DragIcon {
    pub(crate) fn new(
        connection: Rc<XCBConnection>,
        screen: usize,
        context: gpui_wgpu::GpuContext,
        hint: Option<CompositorGpuHint>,
        size: Size<Pixels>,
        scale: f32,
        hotspot: Point<Pixels>,
        scene: &Scene,
    ) -> anyhow::Result<Self> {
        let visuals = find_visuals(&connection, screen);
        let visual = visuals
            .transparent
            .ok_or_else(|| anyhow!("X11 drag icon requires an alpha visual"))?;
        let colormap = connection.generate_id()?;
        connection
            .create_colormap(
                xproto::ColormapAlloc::NONE,
                colormap,
                visuals.root,
                visual.id,
            )?
            .check()?;
        let window = match connection.generate_id() {
            Ok(window) => window,
            Err(error) => {
                let _ = connection.free_colormap(colormap);
                return Err(error.into());
            }
        };
        let mut icon = Self {
            connection,
            window,
            colormap,
            renderer: None,
            hotspot,
            scale,
        };
        let size = size.to_device_pixels(scale);
        icon.connection
            .create_window(
                visual.depth,
                window,
                visuals.root,
                0,
                0,
                size.width.0.max(1).try_into()?,
                size.height.0.max(1).try_into()?,
                0,
                xproto::WindowClass::INPUT_OUTPUT,
                visual.id,
                &xproto::CreateWindowAux::new()
                    .override_redirect(1)
                    .colormap(colormap)
                    .border_pixel(0)
                    .background_pixel(0),
            )?
            .check()?;
        // The preview must never become a pointer target or obscure XDND discovery.
        icon.connection
            .shape_rectangles(
                SO::SET,
                SK::INPUT,
                xproto::ClipOrdering::UNSORTED,
                window,
                0,
                0,
                &[],
            )?
            .check()?;
        let raw = RawWindow {
            connection: as_raw_xcb_connection::AsRawXcbConnection::as_raw_xcb_connection(
                &icon.connection,
            ) as *mut _,
            screen_id: screen,
            window_id: window,
            visual_id: visual.id,
        };
        icon.renderer = Some(WgpuRenderer::new(
            context,
            &raw,
            WgpuSurfaceConfig {
                size,
                transparent: true,
                preferred_present_mode: None,
            },
            hint,
        )?);
        icon.renderer.as_mut().unwrap().draw(scene);
        Ok(icon)
    }

    pub(crate) fn draw(
        &mut self,
        size: Size<Pixels>,
        scale: f32,
        hotspot: Point<Pixels>,
        scene: &Scene,
    ) -> anyhow::Result<()> {
        self.scale = scale;
        self.hotspot = hotspot;
        let size = size.to_device_pixels(scale);
        self.connection.configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .width(size.width.0.max(1) as u32)
                .height(size.height.0.max(1) as u32),
        )?;
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.update_drawable_size(size);
            renderer.draw(scene);
        }
        Ok(())
    }

    pub(crate) fn move_to(&self, x: i16, y: i16) -> anyhow::Result<()> {
        self.connection.configure_window(
            self.window,
            &xproto::ConfigureWindowAux::new()
                .x(x as i32 - (f32::from(self.hotspot.x) * self.scale) as i32)
                .y(y as i32 - (f32::from(self.hotspot.y) * self.scale) as i32)
                .stack_mode(xproto::StackMode::ABOVE),
        )?;
        self.connection.map_window(self.window)?;
        Ok(())
    }
}

impl Drop for X11DragIcon {
    fn drop(&mut self) {
        if let Some(mut renderer) = self.renderer.take() {
            renderer.destroy();
        }
        let _ = self.connection.destroy_window(self.window);
        let _ = self.connection.free_colormap(self.colormap);
        let _ = self.connection.flush();
    }
}
