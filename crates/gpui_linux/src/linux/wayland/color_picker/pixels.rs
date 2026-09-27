use wayland_client::protocol::{wl_output, wl_shm};

pub(super) fn pixel_position(
    x: f64,
    y: f64,
    logical: (u32, u32),
    physical: (u32, u32),
) -> Option<(u32, u32)> {
    if logical.0 == 0
        || logical.1 == 0
        || physical.0 == 0
        || physical.1 == 0
        || !x.is_finite()
        || !y.is_finite()
    {
        return None;
    }
    Some((
        (x * physical.0 as f64 / logical.0 as f64)
            .floor()
            .clamp(0.0, (physical.0 - 1) as f64) as u32,
        (y * physical.1 as f64 / logical.1 as f64)
            .floor()
            .clamp(0.0, (physical.1 - 1) as f64) as u32,
    ))
}

pub(super) fn normalize(
    bytes: &[u8],
    (w, h, stride): (u32, u32, u32),
    format: wl_shm::Format,
    inverted: bool,
    transform: wl_output::Transform,
) -> (Vec<u32>, u32, u32) {
    let t = transform as u32;
    let (dw, dh) = if t % 2 == 1 { (h, w) } else { (w, h) };
    let mut pixels = vec![0; (dw * dh) as usize];
    for y in 0..h {
        for x in 0..w {
            let offset = (y * stride + x * 4) as usize;
            let raw = u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap());
            let ten = matches!(
                format,
                wl_shm::Format::Argb2101010
                    | wl_shm::Format::Xrgb2101010
                    | wl_shm::Format::Abgr2101010
                    | wl_shm::Format::Xbgr2101010
            );
            let (mut r, g, mut b) = if ten {
                (
                    ((raw >> 20) & 1023) * 255 / 1023,
                    ((raw >> 10) & 1023) * 255 / 1023,
                    (raw & 1023) * 255 / 1023,
                )
            } else {
                ((raw >> 16) & 255, (raw >> 8) & 255, raw & 255)
            };
            if matches!(
                format,
                wl_shm::Format::Abgr8888
                    | wl_shm::Format::Xbgr8888
                    | wl_shm::Format::Abgr2101010
                    | wl_shm::Format::Xbgr2101010
            ) {
                std::mem::swap(&mut r, &mut b);
            }
            let sy = if inverted { h - 1 - y } else { y };
            let (mut dx, dy) = match t % 4 {
                1 => (h - 1 - sy, x),
                2 => (w - 1 - x, h - 1 - sy),
                3 => (sy, w - 1 - x),
                _ => (x, sy),
            };
            if t >= 4 {
                dx = dw - 1 - dx;
            }
            pixels[(dy * dw + dx) as usize] = 0xff000000 | r << 16 | g << 8 | b;
        }
    }
    (pixels, dw, dh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_coordinates_follow_fractional_output_scale_and_clamp_edges() {
        assert_eq!(
            pixel_position(800.5, 400.5, (1536, 864), (1920, 1080)),
            Some((1000, 500))
        );
        assert_eq!(
            pixel_position(1536., -0.5, (1536, 864), (1920, 1080)),
            Some((1919, 0))
        );
        assert_eq!(pixel_position(10., 10., (0, 0), (1920, 1080)), None);
    }

    #[test]
    fn capture_conversion_handles_stride_inversion_and_monitor_rotation() {
        let raw: Vec<u8> = [1u32, 2, 3, 0, 4, 5, 6, 0]
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect();
        let (p, w, h) = normalize(
            &raw,
            (3, 2, 16),
            wl_shm::Format::Xrgb8888,
            true,
            wl_output::Transform::_90,
        );
        assert_eq!((w, h), (2, 3));
        assert_eq!(
            p.iter().map(|p| p & 0xffffff).collect::<Vec<_>>(),
            [1, 4, 2, 5, 3, 6]
        );
        let (p, _, _) = normalize(
            &raw,
            (3, 2, 16),
            wl_shm::Format::Xrgb8888,
            false,
            wl_output::Transform::Flipped,
        );
        assert_eq!(
            p.iter().map(|p| p & 0xffffff).collect::<Vec<_>>(),
            [3, 2, 1, 6, 5, 4]
        );
    }

    #[test]
    fn ten_bit_and_reversed_channel_formats_return_opaque_rgb() {
        let raw = (1023u32 | 512 << 10).to_ne_bytes();
        let (p, _, _) = normalize(
            &raw,
            (1, 1, 4),
            wl_shm::Format::Xbgr2101010,
            false,
            wl_output::Transform::Normal,
        );
        assert_eq!(p, [0xffff7f00]);
        let (p, _, _) = normalize(
            &0x00332211u32.to_ne_bytes(),
            (1, 1, 4),
            wl_shm::Format::Abgr8888,
            false,
            wl_output::Transform::Normal,
        );
        assert_eq!(p, [0xff112233]);
    }
}
