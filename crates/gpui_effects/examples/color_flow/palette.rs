pub(super) use gpui_effects::ColorFlowPalette as Palette;
use gpui_effects::ColorFlowPaletteColor;

fn distance(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    (a[0] - b[0]).powi(2) * 0.2126 + (a[1] - b[1]).powi(2) * 0.7152 + (a[2] - b[2]).powi(2) * 0.0722
}

/// Alpha-weighted color clustering of a bounded grid of BGRA image samples.
pub(super) fn extract(
    bytes: &[u8],
    width: usize,
    height: usize,
    grid: usize,
    iterations: usize,
) -> Palette {
    cluster(bytes, width, height, grid, iterations).map(|color| {
        ColorFlowPaletteColor::new(
            gpui::Rgba {
                r: color[0],
                g: color[1],
                b: color[2],
                a: 1.0,
            },
            color[3],
        )
    })
}

fn cluster(
    bytes: &[u8],
    width: usize,
    height: usize,
    grid: usize,
    iterations: usize,
) -> [[f32; 4]; 4] {
    let mut result = [[0.; 4]; 4];
    let Some(length) = width
        .checked_mul(height)
        .and_then(|area| area.checked_mul(4))
    else {
        return result;
    };
    if width == 0 || height == 0 || bytes.len() < length || grid == 0 {
        return result;
    }
    let columns = grid.min(width);
    let rows = grid.min(height);
    let mut samples = Vec::with_capacity(columns * rows);
    let mut average = [0.; 4];
    for row in 0..rows {
        for column in 0..columns {
            let x = (2 * column + 1) * width / (2 * columns);
            let y = (2 * row + 1) * height / (2 * rows);
            let pixel = &bytes[(y * width + x) * 4..][..4];
            let alpha = pixel[3] as f32 / 255.;
            if alpha == 0. {
                continue;
            }
            let sample = [
                pixel[2] as f32 / 255.,
                pixel[1] as f32 / 255.,
                pixel[0] as f32 / 255.,
                alpha,
            ];
            for channel in 0..3 {
                average[channel] += sample[channel] * alpha;
            }
            average[3] += alpha;
            samples.push(sample);
        }
    }
    if samples.is_empty() {
        return result;
    }
    let population = average[3];
    for channel in &mut average[..3] {
        *channel /= population;
    }
    result[0] = *samples
        .iter()
        .min_by(|a, b| distance(a, &average).total_cmp(&distance(b, &average)))
        .unwrap();
    for index in 1..result.len() {
        let separation = |sample: &[f32; 4]| {
            result[..index]
                .iter()
                .map(|center| distance(sample, center))
                .fold(f32::INFINITY, f32::min)
                * sample[3]
        };
        result[index] = *samples
            .iter()
            .max_by(|a, b| separation(a).total_cmp(&separation(b)))
            .unwrap();
    }
    for _ in 0..iterations.max(1) {
        let mut sums = [[0.; 4]; 4];
        for sample in &samples {
            let nearest = (0..4)
                .min_by(|&a, &b| {
                    distance(sample, &result[a]).total_cmp(&distance(sample, &result[b]))
                })
                .unwrap();
            for channel in 0..3 {
                sums[nearest][channel] += sample[channel] * sample[3];
            }
            sums[nearest][3] += sample[3];
        }
        for index in 0..4 {
            if sums[index][3] > 0. {
                for channel in 0..3 {
                    result[index][channel] = sums[index][channel] / sums[index][3];
                }
            }
            result[index][3] = sums[index][3] / population;
        }
    }
    result.sort_by(|a, b| b[3].total_cmp(&a[3]));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_solid_bgra_color_and_ignores_transparent_rgb() {
        let palette = extract(&[0, 0, 255, 255, 255, 0, 0, 0], 2, 1, 32, 8);
        assert_eq!(
            palette[0],
            ColorFlowPaletteColor::new(gpui::rgb(0xff0000), 1.)
        );
        assert_eq!(palette.iter().map(|entry| entry.weight).sum::<f32>(), 1.);
    }

    #[test]
    fn weighted_palette_preserves_color_proportions() {
        let pixels = [
            0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 255, 0, 0, 255,
        ];
        let palette = extract(&pixels, 4, 1, 32, 8);
        assert_eq!(
            palette[0],
            ColorFlowPaletteColor::new(gpui::rgb(0xff0000), 0.75)
        );
        assert_eq!(
            palette[1],
            ColorFlowPaletteColor::new(gpui::rgb(0x0000ff), 0.25)
        );
    }

    #[test]
    fn empty_and_transparent_sources_have_no_color_weight() {
        let empty = [ColorFlowPaletteColor::new(gpui::rgb(0), 0.); 4];
        assert_eq!(extract(&[], 0, 0, 32, 8), empty);
        assert_eq!(extract(&[255, 0, 255, 0], 1, 1, 32, 8), empty);
    }
}
