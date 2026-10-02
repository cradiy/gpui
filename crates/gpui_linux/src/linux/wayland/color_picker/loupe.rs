use anyhow::{Context, Result};
use gpui::{FontRun, PlatformTextSystem, RenderGlyphParams, font, point, px};

pub(super) const LOUPE_W: usize = 122;
pub(super) const LOUPE_H: usize = 150;
const PREVIEW: f32 = 16.;
const PREVIEW_SIZE: f32 = 90.;

struct Glyph {
    mask: Vec<u8>,
    width: usize,
    origin: (i32, i32),
    advance: f32,
}

pub(super) struct Loupe {
    pub scale: usize,
    base: Vec<u32>,
    overlay: Vec<u32>,
    preview: Vec<(usize, i32, i32, f32)>,
    swatch: Vec<(usize, f32)>,
    glyphs: Vec<Glyph>,
}

impl Loupe {
    pub fn new(text: &dyn PlatformTextSystem, scale: usize) -> Result<Self> {
        let mut loupe = Self::chrome(scale);
        let font_id = [
            ".SystemUIFont",
            "Inter",
            "Noto Sans",
            "DejaVu Sans",
            "Liberation Sans",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(text.all_font_names())
        .find_map(|family| {
            let mut descriptor = font(family);
            descriptor.weight = gpui::FontWeight::MEDIUM;
            let id = text.font_id(&descriptor).ok()?;
            "#0123456789ABCDEF"
                .chars()
                .all(|ch| text.glyph_for_char(id, ch).is_some())
                .then_some(id)
        })
        .context("No font is available for the color readout")?;
        for ch in "#0123456789ABCDEF".chars() {
            let glyph_id = text
                .glyph_for_char(font_id, ch)
                .context("Missing color readout glyph")?;
            let params = RenderGlyphParams {
                font_id,
                glyph_id,
                font_size: px(12.),
                subpixel_variant: point(0, 0),
                scale_factor: scale as f32,
                is_emoji: false,
                subpixel_rendering: false,
                dilation: 0,
                blur_radius: 0,
            };
            let bounds = text.glyph_raster_bounds(&params)?;
            let (size, mask) = text.rasterize_glyph(&params, bounds)?;
            let layout = text.layout_line(&ch.to_string(), px(12.), &[FontRun { len: 1, font_id }]);
            loupe.glyphs.push(Glyph {
                mask,
                width: size.width.0 as usize,
                origin: (bounds.origin.x.0, bounds.origin.y.0),
                advance: f32::from(layout.width),
            });
        }
        Ok(loupe)
    }

    fn chrome(scale: usize) -> Self {
        let (w, h) = (LOUPE_W * scale, LOUPE_H * scale);
        let mut base = vec![0; w * h];
        let mut overlay = vec![0; w * h];
        let mut preview = Vec::new();
        let mut swatch = Vec::new();
        let aa = 1. / scale as f32;
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                let p = ((x as f32 + 0.5) * aa, (y as f32 + 0.5) * aa);
                let shadow = rounded_distance((p.0, p.1 - 3.), (10., 10., 102., 130.), 11.).max(0.);
                base[i] = over(0, 0, 0.27 * (-shadow * shadow / 28.).exp());
                let panel = rounded_distance(p, (10., 10., 102., 130.), 11.);
                base[i] = over(base[i], 0x26282d, coverage(panel, aa));
                base[i] = over(base[i], 0xffffff, coverage(panel.abs() - 0.5, aa) * 0.13);
                let clip = coverage(
                    rounded_distance(p, (PREVIEW, PREVIEW, PREVIEW_SIZE, PREVIEW_SIZE), 6.),
                    aa,
                );
                if clip > 0. {
                    let (lx, ly) = (p.0 - PREVIEW, p.1 - PREVIEW);
                    preview.push((
                        i,
                        (lx / 10.).floor() as i32 - 4,
                        (ly / 10.).floor() as i32 - 4,
                        clip,
                    ));
                    let grid_x = (lx + 5.).rem_euclid(10.) - 5.;
                    let grid_y = (ly + 5.).rem_euclid(10.) - 5.;
                    let grid = coverage(grid_x.abs().min(grid_y.abs()) - 0.25, aa) * clip;
                    overlay[i] = over(0, 0x000000, grid * 0.16);
                }
                // A two-tone reticle remains legible over both very light and dark samples.
                let reticle = rounded_distance(p, (55., 55., 12., 12.), 2.);
                overlay[i] = over(
                    overlay[i],
                    0x000000,
                    coverage(reticle.abs() - 1.4, aa) * 0.8,
                );
                overlay[i] = over(overlay[i], 0xffffff, coverage(reticle.abs() - 0.65, aa));
                if p.0 > 18. && p.0 < 104. {
                    base[i] = over(
                        base[i],
                        0xffffff,
                        coverage((p.1 - 112.).abs() - 0.25, aa) * 0.08,
                    );
                }
                let sw = rounded_distance(p, (18., 118., 16., 16.), 4.);
                let sc = coverage(sw, aa);
                if sc > 0. {
                    swatch.push((i, sc));
                }
                overlay[i] = over(overlay[i], 0xffffff, coverage(sw.abs() - 0.5, aa) * 0.25);
            }
        }
        Self {
            scale,
            base,
            overlay,
            preview,
            swatch,
            glyphs: Vec::new(),
        }
    }

    pub fn paint(&self, source: &[u32], w: u32, h: u32, px: u32, py: u32) -> Vec<u32> {
        let mut out = self.base.clone();
        for &(i, dx, dy, coverage) in &self.preview {
            let sx = (px as i32 + dx).clamp(0, w as i32 - 1) as u32;
            let sy = (py as i32 + dy).clamp(0, h as i32 - 1) as u32;
            out[i] = over(out[i], source[(sy * w + sx) as usize], coverage);
        }
        let color = source[(py * w + px) as usize];
        for &(i, coverage) in &self.swatch {
            out[i] = over(out[i], color, coverage);
        }
        for (pixel, overlay) in out.iter_mut().zip(&self.overlay) {
            *pixel = composite(*pixel, *overlay);
        }
        let mut pen = 42. * self.scale as f32;
        let stride = LOUPE_W * self.scale;
        for ch in format!("#{:06X}", color & 0xffffff).chars() {
            let index = ch.to_digit(16).map(|n| n as usize + 1).unwrap_or(0);
            let Some(glyph) = self.glyphs.get(index) else {
                break;
            };
            for (i, alpha) in glyph.mask.iter().enumerate() {
                let x = pen.round() as i32 + glyph.origin.0 + (i % glyph.width) as i32;
                let y = 130 * self.scale as i32 + glyph.origin.1 + (i / glyph.width) as i32;
                if x >= 0 && y >= 0 && x < stride as i32 && y < (LOUPE_H * self.scale) as i32 {
                    let at = y as usize * stride + x as usize;
                    out[at] = over(out[at], 0xf2f3f5, *alpha as f32 / 255.);
                }
            }
            pen += glyph.advance * self.scale as f32;
        }
        out
    }
}

fn rounded_distance(
    (x, y): (f32, f32),
    (left, top, w, h): (f32, f32, f32, f32),
    radius: f32,
) -> f32 {
    let qx = (x - left - w * 0.5).abs() - w * 0.5 + radius;
    let qy = (y - top - h * 0.5).abs() - h * 0.5 + radius;
    qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.) - radius
}
fn coverage(distance: f32, aa: f32) -> f32 {
    (0.5 - distance / aa).clamp(0., 1.)
}
fn over(dst: u32, rgb: u32, opacity: f32) -> u32 {
    let a = (opacity.clamp(0., 1.) * 255.).round() as u32;
    let premul = a << 24
        | (((rgb >> 16) & 255) * a / 255) << 16
        | (((rgb >> 8) & 255) * a / 255) << 8
        | ((rgb & 255) * a / 255);
    composite(dst, premul)
}
fn composite(dst: u32, src: u32) -> u32 {
    let inverse = 255 - (src >> 24);
    let channel =
        |shift: u32| ((src >> shift) & 255) + (((dst >> shift) & 255) * inverse + 127) / 255;
    channel(24) << 24 | channel(16) << 16 | channel(8) << 8 | channel(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn center_and_swatch_preserve_the_selected_pixel_at_all_supported_scales() {
        let source: Vec<u32> = (0..400).map(|i| 0xff000000 | i).collect();
        for scale in 1..=4 {
            let loupe = Loupe::chrome(scale);
            for (x, y) in [(0, 0), (19, 19), (8, 12)] {
                let p = loupe.paint(&source, 20, 20, x, y);
                let at = |x, y| p[y * scale * LOUPE_W * scale + x * scale];
                let expected = source[(y * 20 + x) as usize];
                assert_eq!(at(61, 61), expected);
                assert_eq!(at(26, 126), expected);
                // Every translucent edge and shadow must be premultiplied for wl_shm.
                assert!(p.iter().all(|p| {
                    [p & 255, (p >> 8) & 255, (p >> 16) & 255]
                        .iter()
                        .all(|c| *c <= p >> 24)
                }));
            }
        }
    }
}
