use std::cell::Cell;

use anyhow::{Context as _, Result, ensure};
use gpui::{
    Pixels, Size,
    popup::{PopupAnchor, PopupConstraintAdjustment as Adjustment, PopupGravity, PopupOptions},
};
use windows::Win32::{
    Foundation::{HWND, POINT, RECT},
    Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
    },
    UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};

pub(crate) struct WindowsPopup {
    pub parent: HWND,
    pub options: PopupOptions,
    pub size: Cell<Size<Pixels>>,
}

impl WindowsPopup {
    pub fn bounds(&self) -> Result<RECT> {
        self.bounds_at_scale(None)
    }

    fn bounds_at_scale(&self, popup_scale: Option<f32>) -> Result<RECT> {
        let scale = unsafe { GetDpiForWindow(self.parent) } as f32 / 96.;
        ensure!(scale > 0., "popup parent is no longer available");
        let mut origin = POINT::default();
        unsafe { ClientToScreen(self.parent, &mut origin).ok()? };
        let rect = self.options.anchor_rect;
        let x = origin.x as f32 + rect.origin.x.as_f32() * scale;
        let y = origin.y as f32 + rect.origin.y.as_f32() * scale;
        let width = rect.size.width.as_f32() * scale;
        let height = rect.size.height.as_f32() * scale;
        let popup_size = self.size.get();
        let popup_scale = popup_scale.unwrap_or(scale);
        let size = [
            popup_size.width.as_f32() * popup_scale,
            popup_size.height.as_f32() * popup_scale,
        ];
        let offset = [
            self.options.offset.x.as_f32() * scale,
            self.options.offset.y.as_f32() * scale,
        ];
        ensure!(
            [x, y, width, height, size[0], size[1], offset[0], offset[1]]
                .iter()
                .all(|v| v.is_finite()),
            "popup coordinates must be finite"
        );
        ensure!(
            width >= 0. && height >= 0. && size[0] > 0. && size[1] > 0.,
            "invalid popup dimensions"
        );
        let anchor_rect = RECT {
            left: x.round() as i32,
            top: y.round() as i32,
            right: (x + width).round() as i32,
            bottom: (y + height).round() as i32,
        };
        let monitor = unsafe { MonitorFromRect(&anchor_rect, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetMonitorInfoW(monitor, &mut info).ok()? };
        let anchor = anchor_axes(self.options.anchor);
        let gravity = gravity_axes(self.options.gravity);
        let flags = self.options.constraint_adjustment;
        let horizontal = axis(
            x,
            width,
            size[0],
            offset[0],
            anchor.0,
            gravity.0,
            info.rcWork.left as f32,
            info.rcWork.right as f32,
            flags.contains(Adjustment::FLIP_X),
            flags.contains(Adjustment::SLIDE_X),
            flags.contains(Adjustment::RESIZE_X),
        );
        let vertical = axis(
            y,
            height,
            size[1],
            offset[1],
            anchor.1,
            gravity.1,
            info.rcWork.top as f32,
            info.rcWork.bottom as f32,
            flags.contains(Adjustment::FLIP_Y),
            flags.contains(Adjustment::SLIDE_Y),
            flags.contains(Adjustment::RESIZE_Y),
        );
        Ok(RECT {
            left: horizontal.0.round() as i32,
            top: vertical.0.round() as i32,
            right: (horizontal.0 + horizontal.1).round() as i32,
            bottom: (vertical.0 + vertical.1).round() as i32,
        })
    }

    pub fn place(&self, hwnd: HWND, show: bool) -> Result<()> {
        let popup_scale = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.;
        let rect = self.bounds_at_scale(Some(popup_scale))?;
        unsafe {
            SetWindowPos(
                hwnd,
                None,
                rect.left,
                rect.top,
                (rect.right - rect.left).max(1),
                (rect.bottom - rect.top).max(1),
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .context("positioning anchored popup")?;
            if show {
                let _ = ShowWindow(
                    hwnd,
                    if self.options.grab {
                        SW_SHOW
                    } else {
                        SW_SHOWNOACTIVATE
                    },
                );
                if self.options.grab {
                    let _ = SetForegroundWindow(hwnd);
                }
            }
        }
        Ok(())
    }
}

fn anchor_axes(value: PopupAnchor) -> (i8, i8) {
    use PopupAnchor::*;
    match value {
        Center => (0, 0),
        Top => (0, -1),
        Bottom => (0, 1),
        Left => (-1, 0),
        Right => (1, 0),
        TopLeft => (-1, -1),
        TopRight => (1, -1),
        BottomLeft => (-1, 1),
        BottomRight => (1, 1),
    }
}

fn gravity_axes(value: PopupGravity) -> (i8, i8) {
    use PopupGravity::*;
    match value {
        Center => (0, 0),
        Top => (0, -1),
        Bottom => (0, 1),
        Left => (-1, 0),
        Right => (1, 0),
        TopLeft => (-1, -1),
        TopRight => (1, -1),
        BottomLeft => (-1, 1),
        BottomRight => (1, 1),
    }
}

fn axis(
    start: f32,
    anchor_size: f32,
    size: f32,
    offset: f32,
    anchor: i8,
    gravity: i8,
    min: f32,
    max: f32,
    flip: bool,
    slide: bool,
    resize: bool,
) -> (f32, f32) {
    let position = |a: i8, g: i8| {
        start + anchor_size * (f32::from(a) + 1.) / 2. - size * (1. - f32::from(g)) / 2. + offset
    };
    let fits = |p: f32| p >= min && p + size <= max;
    let mut origin = position(anchor, gravity);
    if !fits(origin) && flip {
        let flipped = position(-anchor, -gravity);
        if fits(flipped) {
            origin = flipped;
        }
    }
    if !fits(origin) && slide {
        origin = origin.clamp(min, (max - size).max(min));
    }
    let mut extent = size;
    if resize && (origin < min || origin + extent > max) {
        let end = (origin + extent).min(max);
        origin = origin.max(min).min(max - 1.);
        extent = (end - origin).max(1.);
    }
    (origin, extent)
}

#[cfg(test)]
mod tests;
