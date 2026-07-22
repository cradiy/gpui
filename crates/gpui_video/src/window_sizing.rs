use gpui::{Bounds, DevicePixels, Pixels, PlatformDisplay, Size, point, px, size};

/// Converts a video's physical display size into a logical window size and
/// scales it down only when it exceeds the supplied usable display area.
pub fn fit_video_window_size(
    video_size: Size<DevicePixels>,
    display_scale_factor: f32,
    maximum_size: Size<Pixels>,
) -> Size<Pixels> {
    let scale_factor = if display_scale_factor.is_finite() && display_scale_factor > 0.0 {
        display_scale_factor
    } else {
        1.0
    };
    let natural_size = video_size.to_pixels(scale_factor);
    if natural_size.width <= px(0.0) || natural_size.height <= px(0.0) {
        return Size::default();
    }

    let width_ratio = (maximum_size.width.as_f32().max(0.0) / natural_size.width.as_f32()).min(1.0);
    let height_ratio =
        (maximum_size.height.as_f32().max(0.0) / natural_size.height.as_f32()).min(1.0);
    let ratio = width_ratio.min(height_ratio);
    size(natural_size.width * ratio, natural_size.height * ratio)
}

/// Returns centered window bounds for a video on the selected display.
///
/// The display's visible area excludes platform taskbars and docks. Videos no
/// larger than that area retain a one-device-pixel-to-one-device-pixel size;
/// larger videos are reduced proportionally without cropping.
pub fn fit_video_window_bounds(
    video_size: Size<DevicePixels>,
    display: &dyn PlatformDisplay,
) -> Bounds<Pixels> {
    let visible = display.visible_bounds();
    let fitted = fit_video_window_size(video_size, display.scale_factor(), visible.size);
    let offset = (visible.size - fitted) / 2.0;
    Bounds::new(
        point(
            visible.origin.x + offset.width,
            visible.origin.y + offset.height,
        ),
        fitted,
    )
}

#[cfg(test)]
mod tests {
    use gpui::{DevicePixels, px, size};

    use super::fit_video_window_size;

    #[test]
    fn smaller_video_keeps_its_native_size() {
        let fitted = fit_video_window_size(
            size(DevicePixels(1280), DevicePixels(720)),
            1.0,
            size(px(1920.0), px(1040.0)),
        );
        assert_eq!(fitted, size(px(1280.0), px(720.0)));
    }

    #[test]
    fn oversized_video_is_reduced_proportionally() {
        let fitted = fit_video_window_size(
            size(DevicePixels(3840), DevicePixels(2160)),
            1.0,
            size(px(1920.0), px(1040.0)),
        );
        assert!((fitted.width.as_f32() - 1848.8889).abs() < 0.001);
        assert_eq!(fitted.height, px(1040.0));
    }

    #[test]
    fn hidpi_video_size_is_converted_before_fitting() {
        let fitted = fit_video_window_size(
            size(DevicePixels(3840), DevicePixels(2160)),
            2.0,
            size(px(1920.0), px(1080.0)),
        );
        assert_eq!(fitted, size(px(1920.0), px(1080.0)));
    }
}
